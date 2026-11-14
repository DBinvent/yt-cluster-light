// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! [i18n] What this crate says, in the language the binary set with
//! `rust_i18n::set_locale` - en, ru, and - not reviewed - fr, de, zh, ja; another language
//! is English. An error's message starts with its code: YT light's codes have three
//! digits (`YT-Ennn`), YT Cluster's own have four.

/// A gRPC client's call refused before it was sent: the connection is not ready. What the
/// generated clients say in tonic's "Service was not ready" place (build.rs puts it there).
pub fn not_ready(err: tonic::codegen::StdError) -> String {
	rust_i18n::t!("grpc.not_ready", err = err).into_owned()
}

/// `not_ready` in a named language, whatever the binary set: for a caller that has one of
/// its own, and for the crates that check the message (ytcluster-types' clients say it too).
pub fn not_ready_in(locale: &str, err: &str) -> String {
	rust_i18n::t!("grpc.not_ready", locale = locale, err = err).into_owned()
}

/// A code's digits when `code` is one of YT light's: `YT-E` and three digits.
pub fn light_code(code: &str) -> Option<u32> {
	let n = code.strip_prefix("YT-E")?;
	(n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// the generated clients say not_ready, not tonic's English
	#[test]
	fn the_generated_clients_say_not_ready() {
		let dir = format!("{}/src/protogen", env!("CARGO_MANIFEST_DIR"));
		let mut clients = 0;
		for f in std::fs::read_dir(&dir).unwrap() {
			let src = std::fs::read_to_string(f.unwrap().path()).unwrap();
			assert!(!src.contains("Service was not ready"));
			clients += src.matches("crate::i18n::not_ready(e.into())").count();
		}
		assert!(clients > 0, "{}", dir);
		let en = not_ready_in("en", "x");
		assert_eq!(en, "YT-E001 the service was not ready: x");
		assert_eq!(not_ready_in("pt", "x"), en, "a language without messages is English");
		assert_ne!(not_ready_in("ru", "x"), en);
	}

	#[test]
	fn it_says_the_reason() {
		assert!(not_ready("no route".into()).ends_with("no route"));
	}

	/// a message that moved here from ytcluster-types kept its code and its languages
	#[test]
	fn an_invalid_action_is_said_under_its_code() {
		assert_eq!(crate::db::DbAction::to_str(99), "YT-E002 An invalid 'action' field value: 99");
		assert_eq!(crate::db::DbAction::to_str(crate::db::DbAction::Update as i32), "update");
	}

	/// a locale file's `key: "message"` lines, in file order
	fn file(l: &str) -> Vec<(String, String)> {
		let p = format!("{}/locales/{}.yml", env!("CARGO_MANIFEST_DIR"), l);
		std::fs::read_to_string(&p).unwrap().lines()
			.filter(|s| !s.starts_with('#') && !s.starts_with('_') && s.contains(": \""))
			.map(|s| {
				let (k, m) = s.split_once(": \"").unwrap();
				(k.trim().to_string(), m.trim_end().trim_end_matches('"').to_string())
			})
			.collect()
	}

	/// [i18n] every message in en, ru, fr, de, zh and ja with the same placeholders, each
	/// starting with its code - three digits, YT-E001 to YT-E029 here, yt-core's from
	/// YT-E030 - the same in every language, and no two messages with one code
	#[test]
	fn every_message_has_its_code_in_every_language() {
		let placeholders = |m: &str| {
			let mut p: Vec<String> = m.split("%{").skip(1).map(|r| r.split('}').next().unwrap().to_string()).collect();
			p.sort();
			p
		};
		let code = |m: &str| m.split(' ').next().filter(|c| c.starts_with("YT-E")).map(str::to_string);
		let en = file("en");
		let mut codes = std::collections::BTreeMap::new();
		for l in ["ru", "fr", "de", "zh", "ja"] {
			let other: std::collections::BTreeMap<String, String> = file(l).into_iter().collect();
			assert_eq!(en.len(), other.len(), "the same keys in en and {}", l);
			for (k, m) in &en {
				let r = other.get(k).unwrap_or_else(|| panic!("{} not in {}", k, l));
				assert_eq!(placeholders(r), placeholders(m), "{} {}", k, l);
				assert_eq!(code(r), code(m), "{}: the code in {}", k, l);
			}
		}
		for (k, m) in &en {
			let c = code(m).unwrap_or_else(|| panic!("{}: no code", k));
			let n = light_code(&c).unwrap_or_else(|| panic!("{}: {} is not three digits - YT Cluster's have four", k, c));
			assert!((1..30).contains(&n), "{}: {} is not this crate's", k, c);
			assert!(codes.insert(c.clone(), k.clone()).is_none(), "{} for {} and {}", c, codes[&c], k);
		}
		assert!(codes.len() >= 2, "{}", codes.len());
	}

	#[test]
	fn a_light_code_has_three_digits() {
		assert_eq!(light_code("YT-E001"), Some(1));
		assert_eq!(light_code("YT-E930"), Some(930));
		assert_eq!(light_code("YT-E1001"), None, "four digits: YT Cluster's");
		assert_eq!(light_code("YT-E01"), None);
		assert_eq!(light_code("SGE-E001"), None);
	}
}
