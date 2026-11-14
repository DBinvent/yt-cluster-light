// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-light's CA, and its TLS to the cluster above. A light node has no az: its CA is its
//! own identity across the link, under its node uuid, as a lower cluster's CA is that
//! cluster's:
//!
//! - **Its own CA**: a P-256 key and a self-signed certificate, made at the first start
//!   and kept in the key directory (`ca.key`, mode 0600, and `ca.crt`). Its fingerprint -
//!   `SHA256:<base64>` of the key - is logged at every start: the operator of the cluster above
//!   compares it with the one shown there and approves it on that cluster's leader, `ytctl
//!   --skv-approve <this node's uuid> --fingerprint <it>`.
//! - **Every sign-in** offers that certificate - the countersigned one once held - its
//!   key's fingerprint as the one live, what it holds of the upper's roots, and the key's
//!   proof over the challenge the last answer handed (`signin::proof_message`).
//! - **Admitted**, the answer carries the CA countersigned by the upper's CA, and the
//!   upper's roots. Taken only when a root is the one pinned at setup - setting
//!   `upper_root`, the `SHA256:...` of the cluster's CA as `ytctl --skv-status` prints it
//!   there - or chains to it, and the countersigned CA is this node's own: kept as
//!   `ca-signed.crt` and `roots.pem`, so a restart needs no approval again.
//! - **From then on every call** to the cluster above is TLS 1.3: a node certificate this CA
//!   issues at the start, its CN this node's uuid, with the countersigned CA beside it; the
//!   far end trusted only when it chains to those roots and names a uuid.
//!
//! What it signs and how the far end checks it are ytserv's `skv/link.rs` and
//! `skv/tls.rs`: the same curve, digest, fingerprint and names.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::sign::Signer;
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode, SslVersion};
use openssl::x509::extension::{
	AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAlternativeName, SubjectKeyIdentifier,
};
use openssl::x509::store::X509StoreBuilder;
use openssl::x509::verify::X509VerifyFlags;
use openssl::x509::{X509, X509Builder, X509NameBuilder, X509Ref};
use tokio::net::TcpStream;
use tokio_openssl::SslStream;
use yt_core_proto::link::LinkGrant;

use crate::core::signin::{proof_message, roots_message};

/// the CA's lifetime, as a cluster's
const CA_DAYS: u32 = 20 * 365;
/// the node certificate's: a new one at every start, and before it runs out
const NODE_DAYS: u32 = 365;
/// a node certificate with fewer days left is issued again
const RENEW_DAYS: i32 = 30;

const CA_KEY: &str = "ca.key";
const CA_CERT: &str = "ca.crt";
const CA_SIGNED: &str = "ca-signed.crt";
const ROOTS: &str = "roots.pem";

fn e<E: std::fmt::Display>(what: &'static str) -> impl Fn(E) -> String {
	move |x| t!("light-tls-internal", what = what, error = x)
}

/// A file's error, said with its path.
fn file_error<E: std::fmt::Display>(path: &Path) -> impl Fn(E) -> String + '_ {
	move |x| t!("light-file", path = path.display(), error = x)
}

/// `SHA256:<base64>` of a key's SPKI DER: the form the cluster above shows its operator.
pub fn fingerprint(spki: &[u8]) -> String {
	format!("SHA256:{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(openssl::sha::sha256(spki)))
}

/// The fingerprint of a certificate's key.
pub fn cert_fp(c: &X509Ref) -> Option<String> {
	Some(fingerprint(&c.public_key().ok()?.public_key_to_der().ok()?))
}

/// Signed by `ca`'s key. The thread's OpenSSL error queue is drained after: a "no" leaves
/// its reason there, and the next TLS read or write on the thread took it for its own.
pub fn signed_by(cert: &X509Ref, ca: &X509Ref) -> bool {
	let r = ca.public_key().ok().is_some_and(|k| cert.verify(&k).unwrap_or(false));
	let _ = openssl::error::ErrorStack::get();
	r
}

fn same_name(a: &openssl::x509::X509NameRef, b: &openssl::x509::X509NameRef) -> bool {
	matches!((a.to_der(), b.to_der()), (Ok(x), Ok(y)) if x == y)
}

/// Issued by itself: subject = issuer, and its own key verifies it.
pub fn self_signed(c: &X509Ref) -> bool {
	same_name(c.issuer_name(), c.subject_name()) && signed_by(c, c)
}

/// The uuid a certificate names (its CN), lowercase and hyphenated.
pub fn cert_uuid(c: &X509Ref) -> Option<String> {
	let cn = c.subject_name().entries_by_nid(Nid::COMMONNAME).next()?;
	let s = cn.data().to_string().ok()?.to_lowercase();
	yt_core_proto::uuid::Uuid::parse_str(&s).ok().map(|u| u.hyphenated().to_string())
}

/// The upper's roots taken from an answer (ytserv's `accepted_roots`, H8): a self-signed
/// root whose key is the pinned one, one held before (`held`), or one a cross-certificate
/// in the answer vouches for - same key and subject, issued by a root already taken. A
/// cross-certificate is never a root itself.
pub fn accepted_roots(certs: &[Vec<u8>], pinned: &str, held: &[Vec<u8>]) -> Vec<Vec<u8>> {
	let parsed: Vec<(Vec<u8>, X509)> = certs.iter().filter_map(|d| X509::from_der(d).ok().map(|c| (d.clone(), c))).collect();
	#[allow(clippy::type_complexity)] // the type says what it holds, as ytserv's
	let (roots, crosses): (Vec<&(Vec<u8>, X509)>, Vec<&(Vec<u8>, X509)>) = parsed.iter().partition(|(_, c)| self_signed(c));
	let mut taken: Vec<&(Vec<u8>, X509)> = roots.iter().copied()
		.filter(|(d, c)| cert_fp(c).as_deref() == Some(pinned.trim()) || held.contains(d)).collect();
	loop {
		let more: Vec<&(Vec<u8>, X509)> = roots.iter().copied()
			.filter(|r| !taken.iter().any(|t| t.0 == r.0))
			.filter(|(_, r)| crosses.iter().any(|(_, x)| cert_fp(x) == cert_fp(r)
				&& same_name(x.subject_name(), r.subject_name())
				&& taken.iter().any(|(_, t)| signed_by(x, t) && same_name(x.issuer_name(), t.subject_name()))))
			.collect();
		if more.is_empty() {
			break;
		}
		taken.extend(more);
	}
	taken.into_iter().map(|(d, _)| d.clone()).collect()
}

fn serial() -> Result<openssl::asn1::Asn1Integer, String> {
	let mut bn = BigNum::new().map_err(e("serial"))?;
	bn.rand(127, MsbOption::MAYBE_ZERO, false).map_err(e("serial"))?;
	bn.to_asn1_integer().map_err(e("serial"))
}

fn not_before() -> Result<Asn1Time, String> {
	// an hour back, for a far end whose clock runs behind
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
	Asn1Time::from_unix(now - 3600).map_err(e("time"))
}

fn p256() -> Result<PKey<Private>, String> {
	let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(e("key"))?;
	PKey::from_ec_key(EcKey::generate(&group).map_err(e("key"))?).map_err(e("key"))
}

/// A new CA for node `uuid`: its key and its self-signed certificate.
fn make_ca(uuid: &str) -> Result<(PKey<Private>, X509), String> {
	let key = p256()?;
	let ca = ca_cert(&key, uuid)?;
	Ok((key, ca))
}

/// The self-signed certificate of CA key `key`, for node `uuid`.
fn ca_cert(key: &PKey<Private>, uuid: &str) -> Result<X509, String> {
	let mut name = X509NameBuilder::new().map_err(e("ca name"))?;
	name.append_entry_by_nid(Nid::ORGANIZATIONNAME, "yt-cluster").map_err(e("ca name"))?;
	name.append_entry_by_nid(Nid::COMMONNAME, &format!("light {} CA", uuid)).map_err(e("ca name"))?;
	let name = name.build();
	let mut b = X509Builder::new().map_err(e("ca cert"))?;
	b.set_version(2).map_err(e("ca cert"))?;
	let (sn, nb, na) = (serial()?, not_before()?, Asn1Time::days_from_now(CA_DAYS).map_err(e("ca cert"))?);
	b.set_serial_number(&sn).map_err(e("ca cert"))?;
	b.set_subject_name(&name).map_err(e("ca cert"))?;
	b.set_issuer_name(&name).map_err(e("ca cert"))?;
	b.set_not_before(&nb).map_err(e("ca cert"))?;
	b.set_not_after(&na).map_err(e("ca cert"))?;
	b.set_pubkey(key).map_err(e("ca cert"))?;
	b.append_extension(BasicConstraints::new().critical().ca().build().map_err(e("ca ext"))?).map_err(e("ca ext"))?;
	b.append_extension(KeyUsage::new().critical().key_cert_sign().crl_sign().build().map_err(e("ca ext"))?).map_err(e("ca ext"))?;
	let ski = SubjectKeyIdentifier::new().build(&b.x509v3_context(None, None)).map_err(e("ca ext"))?;
	b.append_extension(ski).map_err(e("ca ext"))?;
	b.sign(key, MessageDigest::sha256()).map_err(e("ca sign"))?;
	Ok(b.build())
}

/// A node certificate for `key`, issued by the CA: CN and DNS name this node's uuid, for
/// both ends of a connection (as ytserv's `ca::issue`).
fn issue(ca: &X509Ref, ca_key: &PKey<Private>, key: &PKey<Private>, uuid: &str) -> Result<X509, String> {
	let mut name = X509NameBuilder::new().map_err(e("node name"))?;
	name.append_entry_by_nid(Nid::ORGANIZATIONNAME, "yt-cluster").map_err(e("node name"))?;
	name.append_entry_by_nid(Nid::COMMONNAME, uuid).map_err(e("node name"))?;
	let name = name.build();
	let mut b = X509Builder::new().map_err(e("node cert"))?;
	b.set_version(2).map_err(e("node cert"))?;
	let (sn, nb, na) = (serial()?, not_before()?, Asn1Time::days_from_now(NODE_DAYS).map_err(e("node cert"))?);
	b.set_serial_number(&sn).map_err(e("node cert"))?;
	b.set_subject_name(&name).map_err(e("node cert"))?;
	b.set_issuer_name(ca.subject_name()).map_err(e("node cert"))?;
	b.set_not_before(&nb).map_err(e("node cert"))?;
	b.set_not_after(&na).map_err(e("node cert"))?;
	b.set_pubkey(key).map_err(e("node cert"))?;
	b.append_extension(BasicConstraints::new().critical().build().map_err(e("node ext"))?).map_err(e("node ext"))?;
	b.append_extension(KeyUsage::new().critical().digital_signature().build().map_err(e("node ext"))?).map_err(e("node ext"))?;
	b.append_extension(ExtendedKeyUsage::new().server_auth().client_auth().build().map_err(e("node ext"))?).map_err(e("node ext"))?;
	let san = SubjectAlternativeName::new().dns(&format!("{}.yt-cluster", uuid)).build(&b.x509v3_context(Some(ca), None))
		.map_err(e("node ext"))?;
	b.append_extension(san).map_err(e("node ext"))?;
	let aki = AuthorityKeyIdentifier::new().keyid(false).build(&b.x509v3_context(Some(ca), None)).map_err(e("node ext"))?;
	b.append_extension(aki).map_err(e("node ext"))?;
	b.sign(ca_key, MessageDigest::sha256()).map_err(e("node sign"))?;
	Ok(b.build())
}

fn sign(key: &PKey<Private>, msg: &[u8]) -> Result<Vec<u8>, String> {
	let mut s = Signer::new(MessageDigest::sha256(), key).map_err(e("sign"))?;
	s.update(msg).map_err(e("sign"))?;
	s.sign_to_vec().map_err(e("sign"))
}

/// A file of the key directory, written whole or not at all: a temporary file beside it,
/// then renamed over it. `secret`: readable by its owner only.
fn put(dir: &Path, name: &str, data: &[u8], secret: bool) -> Result<(), String> {
	let tmp = dir.join(format!(".{}.tmp", name));
	let _ = std::fs::remove_file(&tmp);
	let mut o = std::fs::OpenOptions::new();
	o.write(true).create_new(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		o.mode(if secret { 0o600 } else { 0o644 });
	}
	// [sqlite] Windows (yt-sqlite): the file takes its directory's ACL - a key_dir under the
	// user's profile, the default, is the user's own
	#[cfg(not(unix))]
	let _ = secret;
	let mut f = o.open(&tmp).map_err(file_error(&tmp))?;
	f.write_all(data).and_then(|_| f.sync_all()).map_err(file_error(&tmp))?;
	std::fs::rename(&tmp, dir.join(name)).map_err(file_error(&dir.join(name)))?;
	// the rename itself, on disk: lost in a power cut, the CA would be made again - a new
	// fingerprint, approved again (review of 2026-11-13). Unix: the directory synced; Windows
	// opens no directory for it
	#[cfg(unix)]
	std::fs::File::open(dir).and_then(|d| d.sync_all()).map_err(file_error(dir))?;
	Ok(())
}

fn read(dir: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
	match std::fs::read(dir.join(name)) {
		Ok(d) => Ok(Some(d)),
		Err(x) if x.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(x) => Err(file_error(&dir.join(name))(x)),
	}
}

/// What a sign-in offers of the CA (`LinkSignIn`'s `ca_*` fields).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Offer {
	pub cert: Vec<u8>,
	pub live: Vec<String>,
	pub roots_hash: Vec<u8>,
	pub proof: Vec<u8>,
}

/// What the cluster above handed and this node took.
#[derive(Default)]
struct Held {
	/// this CA, countersigned by the upper's
	signed: Option<X509>,
	/// the upper's roots, DER
	roots: Vec<Vec<u8>>,
}

/// The CA of a light node and what it took from the cluster above.
pub struct Tls {
	dir: PathBuf,
	/// the root of the cluster above pinned at setup (setting `upper_root`)
	pin: Option<String>,
	key: PKey<Private>,
	ca: X509,
	fp: String,
	uuid: String,
	/// this node's TLS key and certificate: issued again when it nears its end (`dial`)
	node: Mutex<(PKey<Private>, X509)>,
	held: Mutex<Held>,
	/// how many times what is held changed - a countersigned CA, the roots: a connection
	/// made before the last change is made again (calls.rs keeps one per generation)
	generation: std::sync::atomic::AtomicU64,
	/// what changed or went wrong at the last answer, for the log
	news: Mutex<Option<Result<String, String>>>,
}

impl Tls {
	/// The CA kept in `dir`, made there at the first start; `uuid` this node's, `pin` the
	/// root of the cluster above pinned at setup.
	pub fn open(dir: &Path, uuid: &str, pin: Option<&str>) -> Result<Tls, String> {
		// 0700 on a directory made here only: a key_dir the operator named keeps its mode -
		// the key in it is 0600 anyway (review of 2026-11-13)
		if !dir.exists() {
			std::fs::create_dir_all(dir).map_err(file_error(dir))?;
			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt;
				std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(file_error(dir))?;
			}
		}
		let (key, ca) = match (read(dir, CA_KEY)?, read(dir, CA_CERT)?) {
			(Some(k), Some(c)) => {
				let key = PKey::private_key_from_pem(&k).map_err(e("ca.key"))?;
				let ca = X509::from_pem(&c).map_err(e("ca.crt"))?;
				let mine = key.public_key_to_der().map_err(e("ca.key"))?;
				if ca.public_key().and_then(|p| p.public_key_to_der()).ok() != Some(mine) {
					return Err(t!("light-ca-mismatch", dir = dir.display()));
				}
				(key, ca)
			}
			(None, None) => {
				let (key, ca) = make_ca(uuid)?;
				put(dir, CA_KEY, &key.private_key_to_pem_pkcs8().map_err(e("ca.key"))?, true)?;
				put(dir, CA_CERT, &ca.to_pem().map_err(e("ca.crt"))?, false)?;
				(key, ca)
			}
			// a start stopped between the two writes: the certificate again, of the same
			// key - the fingerprint, and so the approval above, stay (review of 2026-11-13)
			(Some(k), None) => {
				let key = PKey::private_key_from_pem(&k).map_err(e("ca.key"))?;
				let ca = ca_cert(&key, uuid)?;
				put(dir, CA_CERT, &ca.to_pem().map_err(e("ca.crt"))?, false)?;
				(key, ca)
			}
			(None, Some(_)) => return Err(t!("light-ca-key-lost", dir = dir.display())),
		};
		let fp = cert_fp(&ca).ok_or_else(|| t!("light-ca-no-key"))?;
		let mut held = Held::default();
		if let Some(d) = read(dir, CA_SIGNED)? {
			let c = X509::from_pem(&d).map_err(e("ca-signed.crt"))?;
			// the CA's own, or nothing: a file from another key is not offered
			if cert_fp(&c).as_deref() == Some(fp.as_str()) && same_name(c.subject_name(), ca.subject_name()) {
				held.signed = Some(c);
			}
		}
		if let Some(d) = read(dir, ROOTS)? {
			held.roots = X509::stack_from_pem(&d).map_err(e("roots.pem"))?.iter().filter_map(|c| c.to_der().ok()).collect();
		}
		let node_key = p256()?;
		let node_cert = issue(&ca, &key, &node_key, uuid)?;
		let pin = pin.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
		Ok(Tls { dir: dir.to_path_buf(), pin, key, ca, fp, uuid: uuid.to_string(), node: Mutex::new((node_key, node_cert)),
			held: Mutex::new(held), generation: std::sync::atomic::AtomicU64::new(0), news: Mutex::new(None) })
	}

	/// This node's TLS key and certificate - issued again once fewer than RENEW_DAYS of it
	/// are left: a node that runs for a year keeps its calls (review of 2026-11-13).
	fn node_cert(&self) -> Result<(PKey<Private>, X509), String> {
		let mut n = self.node.lock().unwrap_or_else(|x| x.into_inner());
		let left = Asn1Time::days_from_now(0).and_then(|now| now.diff(n.1.not_after())).map_err(e("node cert"))?;
		if left.days < RENEW_DAYS {
			let k = p256()?;
			let c = issue(&self.ca, &self.key, &k, &self.uuid)?;
			*n = (k, c);
		}
		Ok(n.clone())
	}

	/// The fingerprint of this node's CA key: what the operator above approves.
	pub fn fingerprint(&self) -> &str {
		&self.fp
	}

	pub fn dir(&self) -> &Path {
		&self.dir
	}

	pub fn pinned(&self) -> bool {
		self.pin.is_some()
	}

	/// Which change of what is held this is: it moves when a countersigned CA or the roots
	/// are taken (`take`), so a kept connection is known to be older than the chain it
	/// would present now (review of 2026-11-13).
	pub fn generation(&self) -> u64 {
		self.generation.load(std::sync::atomic::Ordering::Relaxed)
	}

	/// Countersigned and the roots held: every call to the cluster above is TLS.
	pub fn ready(&self) -> bool {
		let h = self.held.lock().unwrap_or_else(|x| x.into_inner());
		h.signed.is_some() && !h.roots.is_empty()
	}

	/// For yt_info: where the CA stands.
	pub fn status(&self) -> String {
		let h = self.held.lock().unwrap_or_else(|x| x.into_inner());
		match (&h.signed, h.roots.len()) {
			(Some(_), n) if n > 0 => t!("light-tls-status-ready", count = n),
			_ if self.pin.is_none() => t!("light-tls-status-no-pin"),
			_ => t!("light-tls-status-waiting"),
		}
	}

	/// What a sign-in offers: the CA as held, its key as the one live, the roots' hash, and
	/// the key's proof over `challenge` (none without one) for this node, `id` - its uuid,
	/// where a cluster's proof names its az: a light node has none.
	pub fn offer(&self, id: &str, challenge: &[u8]) -> Offer {
		let h = self.held.lock().unwrap_or_else(|x| x.into_inner());
		let mine = h.signed.as_ref().unwrap_or(&self.ca);
		let proof = if challenge.is_empty() { Vec::new() } else {
			sign(&self.key, &proof_message(challenge, id, &self.fp)).unwrap_or_default()
		};
		Offer {
			cert: mine.to_der().unwrap_or_default(),
			live: vec![self.fp.clone()],
			roots_hash: openssl::sha::sha256(&roots_message(&h.roots)).to_vec(),
			proof,
		}
	}

	/// An answer's countersigned CA and roots, taken when they chain to the pin; what
	/// changed, or why nothing was taken, waits in `news`.
	pub fn take(&self, g: &LinkGrant) {
		if g.ca_signed.is_empty() && g.ca_roots.is_empty() {
			return;
		}
		let r = self.take_certs(g);
		let mut n = self.news.lock().unwrap_or_else(|x| x.into_inner());
		match r {
			Ok(Some(t)) => {
				self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
				*n = Some(Ok(t));
			}
			Ok(None) => {}
			Err(x) => *n = Some(Err(x)),
		}
	}

	fn take_certs(&self, g: &LinkGrant) -> Result<Option<String>, String> {
		let Some(pin) = self.pin.as_deref() else {
			return Err(t!("light-root-not-pinned"));
		};
		let mut h = self.held.lock().unwrap_or_else(|x| x.into_inner());
		let given = if g.ca_roots.is_empty() { h.roots.clone() } else { g.ca_roots.clone() };
		let roots = accepted_roots(&given, pin, &h.roots);
		if roots.is_empty() {
			return Err(t!("light-root-no-chain", pin = pin));
		}
		let mut what = Vec::new();
		if !g.ca_signed.is_empty() && h.signed.as_ref().and_then(|c| c.to_der().ok()).as_deref() != Some(&g.ca_signed[..]) {
			let signed = X509::from_der(&g.ca_signed).map_err(e("the countersigned CA"))?;
			let ours = cert_fp(&signed).as_deref() == Some(self.fp.as_str()) && same_name(signed.subject_name(), self.ca.subject_name());
			let chained = roots.iter().filter_map(|d| X509::from_der(d).ok()).any(|root| signed_by(&signed, &root));
			if !ours || !chained {
				return Err(t!("light-ca-not-ours"));
			}
			put(&self.dir, CA_SIGNED, &signed.to_pem().map_err(e("ca-signed.crt"))?, false)?;
			h.signed = Some(signed);
			what.push(t!("light-tls-countersigned"));
		}
		if !g.ca_roots.is_empty() && roots != h.roots {
			let mut pem = Vec::new();
			for d in &roots {
				pem.extend(X509::from_der(d).and_then(|c| c.to_pem()).map_err(e("roots.pem"))?);
			}
			put(&self.dir, ROOTS, &pem, false)?;
			what.push(t!("light-tls-roots-held", count = roots.len()));
			h.roots = roots;
		}
		Ok(if what.is_empty() { None } else { Some(what.join(", ")) })
	}

	/// What changed or went wrong since last asked.
	pub fn news(&self) -> Option<Result<String, String>> {
		self.news.lock().unwrap_or_else(|x| x.into_inner()).take()
	}

	/// TLS 1.3 to `addr` (host:port), a node of the cluster above: this node's certificate and
	/// its countersigned CA; the far end's chain to the roots held, naming a uuid. The
	/// server's HTTP/2 preface is left for the client.
	pub async fn dial(&self, addr: &str, timeout: Duration) -> std::io::Result<SslStream<TcpStream>> {
		let io = |x: &dyn std::fmt::Display| std::io::Error::other(x.to_string());
		let (signed, roots) = {
			let h = self.held.lock().unwrap_or_else(|x| x.into_inner());
			(h.signed.clone(), h.roots.clone())
		};
		let (node_key, node_cert) = self.node_cert().map_err(|x| io(&x))?;
		let ssl = {
			let mut b = SslConnector::builder(SslMethod::tls_client()).map_err(|x| io(&x))?;
			b.set_min_proto_version(Some(SslVersion::TLS1_3)).map_err(|x| io(&x))?;
			b.set_private_key(&node_key).map_err(|x| io(&x))?;
			b.set_certificate(&node_cert).map_err(|x| io(&x))?;
			b.check_private_key().map_err(|x| io(&x))?;
			if let Some(c) = signed {
				b.add_extra_chain_cert(c).map_err(|x| io(&x))?;
			}
			// the upper's roots and nothing else: the system's trust store vouches for no node
			let mut store = X509StoreBuilder::new().map_err(|x| io(&x))?;
			for r in &roots {
				store.add_cert(X509::from_der(r).map_err(|x| io(&x))?).map_err(|x| io(&x))?;
			}
			store.set_flags(X509VerifyFlags::PARTIAL_CHAIN).map_err(|x| io(&x))?;
			b.set_cert_store(store.build());
			b.set_verify_callback(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT, |ok, ctx| {
				ok && (ctx.error_depth() != 0 || ctx.current_cert().and_then(cert_uuid).is_some())
			});
			b.set_alpn_protos(b"\x02h2").map_err(|x| io(&x))?;
			let mut cfg = b.build().configure().map_err(|x| io(&x))?;
			cfg.set_verify_hostname(false);
			// no version named: the far end answers with its oldest live certificate
			cfg.into_ssl("yt-cluster").map_err(|x| io(&x))?
		};
		let t = tokio::time::timeout(timeout, TcpStream::connect(addr)).await.map_err(|x| io(&x))??;
		let _ = t.set_nodelay(true);
		let mut s = SslStream::new(ssl, t).map_err(|x| io(&x))?;
		let _ = openssl::error::ErrorStack::get();
		tokio::time::timeout(timeout, Pin::new(&mut s).connect()).await.map_err(|x| io(&x))?.map_err(|x| io(&x))?;
		Ok(s)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::signin::proof_message;
	use openssl::sign::Verifier;

	fn dir(tag: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("yt-light-tls-{}-{}", tag, std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		d
	}

	const UUID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";

	/// The upper's side, as ytserv's skv/link.rs `countersign` makes it
	fn countersign(upper: &X509Ref, upper_key: &PKey<Private>, lower: &X509Ref) -> Vec<u8> {
		let mut b = X509Builder::new().unwrap();
		b.set_version(2).unwrap();
		b.set_serial_number(&serial().unwrap()).unwrap();
		b.set_subject_name(lower.subject_name()).unwrap();
		b.set_issuer_name(upper.subject_name()).unwrap();
		b.set_not_before(&not_before().unwrap()).unwrap();
		b.set_not_after(&Asn1Time::days_from_now(30).unwrap()).unwrap();
		b.set_pubkey(&lower.public_key().unwrap()).unwrap();
		b.append_extension(BasicConstraints::new().critical().ca().build().unwrap()).unwrap();
		b.sign(upper_key, MessageDigest::sha256()).unwrap();
		b.build().to_der().unwrap()
	}

	#[test]
	fn the_ca_is_made_once_and_kept() {
		let d = dir("kept");
		let a = Tls::open(&d, UUID, None).unwrap();
		let b = Tls::open(&d, UUID, None).unwrap();
		assert_eq!(a.fingerprint(), b.fingerprint(), "a restart keeps the CA: no approval again");
		assert!(a.fingerprint().starts_with("SHA256:") && a.fingerprint().len() == 7 + 43);
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(std::fs::metadata(d.join(CA_KEY)).unwrap().permissions().mode() & 0o777, 0o600);
		}
		let (_, nc) = a.node_cert().unwrap();
		assert_eq!(cert_uuid(&nc).as_deref(), Some(UUID), "the node certificate names the node");
		assert!(signed_by(&nc, &a.ca) && self_signed(&a.ca));
		// a start stopped between the key and the certificate: the same CA again
		std::fs::remove_file(d.join(CA_CERT)).unwrap();
		let c = Tls::open(&d, UUID, None).unwrap();
		assert_eq!(c.fingerprint(), a.fingerprint(), "the certificate made again from the key: the approval stays");
		assert!(d.join(CA_CERT).exists());
		// the key lost: refused, never replaced unasked
		std::fs::remove_file(d.join(CA_KEY)).unwrap();
		assert!(Tls::open(&d, UUID, None).err().is_some_and(|x| x.contains("CA key is lost")));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	#[cfg(unix)]
	fn a_key_dir_the_operator_made_keeps_its_mode() {
		use std::os::unix::fs::PermissionsExt;
		let d = dir("mode");
		std::fs::create_dir_all(&d).unwrap();
		std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o750)).unwrap();
		Tls::open(&d, UUID, None).unwrap();
		assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o750);
		let made = dir("made");
		Tls::open(&made, UUID, None).unwrap();
		assert_eq!(std::fs::metadata(&made).unwrap().permissions().mode() & 0o777, 0o700, "made here: 0700");
		let _ = std::fs::remove_dir_all(&d);
		let _ = std::fs::remove_dir_all(&made);
	}

	#[test]
	fn a_node_certificate_near_its_end_is_issued_again() {
		let d = dir("renew");
		let t = Tls::open(&d, UUID, None).unwrap();
		let (_, first) = t.node_cert().unwrap();
		let (_, same) = t.node_cert().unwrap();
		assert_eq!(first.to_der().unwrap(), same.to_der().unwrap(), "a year left: kept");
		// one with a day left, as a long run leaves it
		{
			let k = p256().unwrap();
			let mut b = X509Builder::new().unwrap();
			b.set_version(2).unwrap();
			b.set_serial_number(&serial().unwrap()).unwrap();
			b.set_subject_name(first.subject_name()).unwrap();
			b.set_issuer_name(t.ca.subject_name()).unwrap();
			b.set_not_before(&not_before().unwrap()).unwrap();
			b.set_not_after(&Asn1Time::days_from_now(1).unwrap()).unwrap();
			b.set_pubkey(&k).unwrap();
			b.sign(&t.key, MessageDigest::sha256()).unwrap();
			*t.node.lock().unwrap() = (k, b.build());
		}
		let (_, renewed) = t.node_cert().unwrap();
		let left = Asn1Time::days_from_now(0).unwrap().diff(renewed.not_after()).unwrap();
		assert!(left.days > RENEW_DAYS, "issued again: {} days", left.days);
		assert_eq!(cert_uuid(&renewed).as_deref(), Some(UUID));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn the_proof_signs_the_challenge_for_this_az_with_the_ca_key() {
		let d = dir("proof");
		let t = Tls::open(&d, UUID, None).unwrap();
		let none = t.offer(UUID, b"");
		assert!(none.proof.is_empty(), "no challenge yet: no proof");
		assert_eq!(none.live, vec![t.fingerprint().to_string()]);
		assert_eq!(none.roots_hash, openssl::sha::sha256(&roots_message(&[])).to_vec());
		let o = t.offer(UUID, b"challenge");
		let ca = X509::from_der(&o.cert).unwrap();
		let k = ca.public_key().unwrap();
		let mut v = Verifier::new(MessageDigest::sha256(), &k).unwrap();
		v.update(&proof_message(b"challenge", UUID, t.fingerprint())).unwrap();
		assert!(v.verify(&o.proof).unwrap(), "the upper's proof_verifies holds");
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn a_countersigned_ca_is_taken_only_through_the_pinned_root() {
		let (ukey, upper) = make_ca("aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa").unwrap();
		let pin = cert_fp(&upper).unwrap();
		let d = dir("take");
		let t = Tls::open(&d, UUID, Some(&pin)).unwrap();
		assert!(!t.ready());
		let grant = LinkGrant { granted: true, ca_signed: countersign(&upper, &ukey, &t.ca),
			ca_roots: vec![upper.to_der().unwrap()], ..Default::default() };
		// another root than the pinned one: nothing taken
		let (okey, other) = make_ca("cccccccc-cccc-5ccc-8ccc-cccccccccccc").unwrap();
		let wrong = LinkGrant { granted: true, ca_signed: countersign(&other, &okey, &t.ca),
			ca_roots: vec![other.to_der().unwrap()], ..Default::default() };
		t.take(&wrong);
		assert!(!t.ready() && matches!(t.news(), Some(Err(x)) if x.contains("pinned")));
		assert_eq!(t.generation(), 0, "a refused answer changes nothing held");
		t.take(&grant);
		assert!(t.ready(), "{:?}", t.news());
		assert!(matches!(t.news(), Some(Ok(x)) if x.contains("countersigned")));
		assert_eq!(t.generation(), 1, "what is held changed: a kept connection is made again");
		t.take(&grant);
		assert!(t.news().is_none(), "the same answer again changes nothing");
		assert_eq!(t.generation(), 1);
		// kept: a restart offers the countersigned CA, with the roots' hash
		let again = Tls::open(&d, UUID, Some(&pin)).unwrap();
		assert!(again.ready());
		let o = again.offer(UUID, b"c");
		assert_eq!(o.cert, grant.ca_signed);
		assert_eq!(o.roots_hash, openssl::sha::sha256(&roots_message(&grant.ca_roots)).to_vec());
		// no pin: nothing taken, and said
		let n = Tls::open(&dir("nopin"), UUID, None).unwrap();
		n.take(&grant);
		assert!(!n.ready() && matches!(n.news(), Some(Err(x)) if x.contains("upper_root")));
		let _ = std::fs::remove_dir_all(&d);
		let _ = std::fs::remove_dir_all(dir("nopin"));
	}

	#[test]
	fn a_countersigned_ca_of_another_key_is_refused() {
		let (ukey, upper) = make_ca("aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa").unwrap();
		let d = dir("other");
		let t = Tls::open(&d, UUID, cert_fp(&upper).as_deref()).unwrap();
		let (_, stranger) = make_ca(UUID).unwrap(); // same subject, another key
		t.take(&LinkGrant { ca_signed: countersign(&upper, &ukey, &stranger), ca_roots: vec![upper.to_der().unwrap()],
			..Default::default() });
		assert!(!t.ready());
		assert!(matches!(t.news(), Some(Err(x)) if x.contains("not this node's")));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn a_rotated_root_is_taken_through_its_cross_certificate() {
		let (k1, r1) = make_ca("aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa").unwrap();
		let (_, r2) = make_ca("aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaab").unwrap();
		let cross = countersign(&r1, &k1, &r2);
		let (d1, d2) = (r1.to_der().unwrap(), r2.to_der().unwrap());
		let pin = cert_fp(&r1).unwrap();
		assert_eq!(accepted_roots(std::slice::from_ref(&d2), &pin, &[]), Vec::<Vec<u8>>::new(), "no chain to the pin: none");
		assert_eq!(accepted_roots(&[d1.clone(), d2.clone(), cross.clone()], &pin, &[]), vec![d1.clone(), d2.clone()]);
		assert_eq!(accepted_roots(&[d2.clone(), cross], "SHA256:other", std::slice::from_ref(&d1)), Vec::<Vec<u8>>::new(),
			"a held root vouches only when it is in the answer too");
		assert_eq!(accepted_roots(std::slice::from_ref(&d1), "SHA256:other", std::slice::from_ref(&d1)), vec![d1]);
	}
}
