// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! [i18n] What the lite node says, in one language: en, ru, and - not reviewed - fr,
//! de, zh, ja (`locales/`). An error's message starts with its code, the same in every
//! language - after its SQLSTATE when it has one, which the extension reads at position
//! 0. YT lite's codes have three digits (`YT-Ennn`: yt-core-proto YT-E001 to YT-E029,
//! this crate YT-E030 and up); YT Cluster's own have four.
//!
//! The shared link files (`core/pos.rs`, `route.rs`, `relay.rs`, `signin.rs`) say theirs
//! with `rust_i18n::t!` spelled out, so the same line compiles in ytserv - which reads
//! these locale files too, by symlink, beside its own.

/// The languages this crate has messages in.
pub const LANGS: [&str; 6] = ["en", "ru", "fr", "de", "zh", "ja"];

/// The language a locale names, when this crate has messages in it: `ru_RU.UTF-8`,
/// `ru-RU` and `ru` are `ru`; `C`, `POSIX`, nothing, or a language without messages is
/// none.
pub fn language_of(locale: &str) -> Option<&'static str> {
	let l: String = locale.trim().chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_lowercase();
	LANGS.iter().copied().find(|x| *x == l)
}

/// The language of yt-lite's messages: its `lang` setting, else the database's
/// (`lc_messages`), else the system locale's, else English.
pub fn pick(setting: Option<&str>, database: Option<&str>, system: Option<&str>) -> &'static str {
	[setting, database, system].into_iter().flatten().find_map(language_of).unwrap_or("en")
}

/// The system locale, as the environment names it: `LC_ALL`, else `LC_MESSAGES`, else
/// `LANG`.
pub fn system_locale() -> Option<String> {
	["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.trim().is_empty())
}

/// Say everything in `lang` from now on.
pub fn set(lang: &str) {
	rust_i18n::set_locale(lang);
}

/// A message's SQLSTATE, when it leads with one: five of [0-9A-Z], one a digit, then ": ".
pub fn sqlstate(m: &str) -> Option<&str> {
	let b = m.as_bytes();
	(b.len() > 7 && b[..5].iter().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
		&& b[..5].iter().any(|c| c.is_ascii_digit()) && &b[5..7] == b": ").then(|| &m[..5])
}

/// The code a message starts with, after its SQLSTATE when it has one.
pub fn code(m: &str) -> Option<&str> {
	let rest = match sqlstate(m) { Some(_) => &m[7..], None => m };
	rest.split(' ').next().filter(|c| c.starts_with("YT-E"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::{BTreeMap, BTreeSet};

	#[test]
	fn the_language_is_the_setting_s_then_the_database_s_then_the_system_s() {
		assert_eq!(language_of("ru_RU.UTF-8"), Some("ru"));
		assert_eq!(language_of("de-DE"), Some("de"));
		assert_eq!(language_of(" JA "), Some("ja"));
		assert_eq!(language_of("C.UTF-8"), None);
		assert_eq!(language_of("POSIX"), None);
		assert_eq!(language_of("pt_BR.UTF-8"), None, "no messages in it");
		assert_eq!(language_of(""), None);
		assert_eq!(pick(Some("fr"), Some("ru_RU.UTF-8"), Some("de_DE")), "fr");
		assert_eq!(pick(None, Some("ru_RU.UTF-8"), Some("de_DE")), "ru");
		assert_eq!(pick(Some("pt"), Some("C"), Some("de_DE.UTF-8")), "de", "one without messages is passed over");
		assert_eq!(pick(None, Some("C.UTF-8"), None), "en");
		assert_eq!(pick(None, None, None), "en");
	}

	/// a locale file's `key: "message"` lines, in file order
	fn file(l: &str) -> Vec<(String, String)> {
		let p = format!("{}/locales/{}.yml", env!("CARGO_MANIFEST_DIR"), l);
		std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {}", p, e)).lines()
			.filter(|s| !s.starts_with('#') && !s.starts_with('_') && s.contains(": \""))
			.map(|s| {
				let (k, m) = s.split_once(": \"").unwrap();
				(k.trim().to_string(), m.trim_end().trim_end_matches('"').to_string())
			})
			.collect()
	}

	fn placeholders(m: &str) -> Vec<String> {
		let mut p: Vec<String> = m.split("%{").skip(1).map(|r| r.split('}').next().unwrap().to_string()).collect();
		p.sort();
		p
	}

	/// every message in every language, with the same placeholders, and a message that
	/// spans lines (`\n`) with as many lines
	#[test]
	fn every_message_is_in_every_language_with_the_same_placeholders() {
		let en = file("en");
		let en_keys: BTreeSet<&String> = en.iter().map(|(k, _)| k).collect();
		assert_eq!(en_keys.len(), en.len(), "a key twice in en");
		for l in &LANGS[1..] {
			let other: BTreeMap<String, String> = file(l).into_iter().collect();
			let only: Vec<&String> = other.keys().filter(|k| !en_keys.contains(k)).collect();
			assert!(only.is_empty(), "in {} only: {:?}", l, only);
			for (k, m) in &en {
				let r = other.get(k).unwrap_or_else(|| panic!("{} not in {}", k, l));
				assert!(!r.is_empty(), "{}: empty in {}", k, l);
				assert_eq!(placeholders(r), placeholders(m), "{} {}", k, l);
				assert_eq!(r.matches("\\n").count(), m.matches("\\n").count(), "{}: the lines in {}", k, l);
			}
		}
	}

	/// an error's code: the same in every language, on one message, with three digits -
	/// YT lite's, YT-E030 and up here - and its SQLSTATE at position 0 everywhere
	#[test]
	fn an_errors_code_is_the_same_in_every_language_and_used_once() {
		let en = file("en");
		for l in &LANGS[1..] {
			let other: BTreeMap<String, String> = file(l).into_iter().collect();
			for (k, m) in &en {
				assert_eq!(code(&other[k]), code(m), "{}: the code in {}", k, l);
				assert_eq!(sqlstate(&other[k]), sqlstate(m), "{}: the SQLSTATE in {}", k, l);
			}
		}
		let mut codes = BTreeMap::new();
		for (k, m) in &en {
			if let Some(c) = code(m) {
				let n = yt_core_proto::i18n::lite_code(c)
					.unwrap_or_else(|| panic!("{}: {} is not three digits - YT Cluster's have four", k, c));
				assert!(n >= 30, "{}: {} is yt-core-proto's range", k, c);
				assert!(codes.insert(c.to_string(), k.clone()).is_none(), "{} for {} and {}", c, codes[c], k);
				// a fragment carries no code: after it comes the message, not punctuation
				let rest = match sqlstate(m) { Some(_) => &m[7..], None => m.as_str() };
				assert!(rest[c.len()..].starts_with(' ') && !rest[c.len() + 1..].starts_with([',', ';', ':', '-', '(', ')', '.']),
					"{}: a fragment with the code {}", k, c);
			}
		}
		assert!(codes.len() >= 40, "{}", codes.len());
		assert_eq!(sqlstate("0A000: YT-E056 x"), Some("0A000"));
		assert_eq!(code("0A000: YT-E056 x"), Some("YT-E056"));
		assert_eq!(code("YT-E030 x"), Some("YT-E030"));
		assert_eq!(code("link TLS: x"), None);
		assert_eq!(sqlstate("ABORT: x"), None, "a word, not a SQLSTATE");
	}

	/// every `.rs` file under `dir`, with its text
	fn sources(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
		for e in std::fs::read_dir(dir).unwrap() {
			let p = e.unwrap().path();
			if p.is_dir() {
				sources(&p, out);
			} else if p.extension().is_some_and(|x| x == "rs") {
				out.push((p.display().to_string(), std::fs::read_to_string(&p).unwrap()));
			}
		}
	}

	/// the keys a source file asks for: `t!("key"`, with or without `rust_i18n::` before it
	fn asked(src: &str) -> Vec<String> {
		src.split("t!(\"").skip(1).filter_map(|r| r.split('"').next()).map(str::to_string).collect()
	}

	/// a key the code asks for is in the files - a missing one is said as its own name, in
	/// every language - and a message nothing asks for is not kept
	#[test]
	fn the_code_and_the_files_name_the_same_messages() {
		let keys: BTreeSet<String> = file("en").into_iter().map(|(k, _)| k).collect();
		let mut src = Vec::new();
		sources(std::path::Path::new(&format!("{}/src", env!("CARGO_MANIFEST_DIR"))), &mut src);
		let mut used = BTreeSet::new();
		for (path, text) in &src {
			for k in asked(text) {
				// a test's own strings aside: a key is letters, digits, `-` and `.`
				if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
					continue;
				}
				if k.starts_with("lite-") || k.starts_with("link-") {
					assert!(keys.contains(&k), "{}: t!(\"{}\") is in no locale file", path, k);
				}
				used.insert(k);
			}
		}
		let unused: Vec<&String> = keys.iter().filter(|k| !used.contains(*k)).collect();
		assert!(unused.is_empty(), "in the locale files, asked for by nothing: {:?}", unused);
	}

	/// one message in each language, to see the files are the ones compiled in
	#[test]
	fn a_message_is_said_in_the_language_asked_for() {
		let en = rust_i18n::t!("lite-info-lite", locale = "en").into_owned();
		assert_eq!(en, "YT lite: one database, a leaf of a YT cluster");
		for l in &LANGS[1..] {
			assert_ne!(rust_i18n::t!("lite-info-az-untold", locale = *l).into_owned(),
				rust_i18n::t!("lite-info-az-untold", locale = "en").into_owned(), "{}", l);
		}
		assert_eq!(rust_i18n::t!("lite-info-lite", locale = "pt").into_owned(), en, "a language without messages is English");
		let m = rust_i18n::t!("lite-one-database", locale = "ru", db = "appdb").into_owned();
		assert!(m.starts_with("0A000: YT-E056 ") && m.contains("appdb"), "{}", m);
	}
}
