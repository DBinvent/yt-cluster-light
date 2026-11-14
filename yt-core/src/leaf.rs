// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The link's turns for a light node inside an application: once a second it signs in -
//! when it is not let in, every minute, and after a change of host - then takes what came
//! down and sends what goes up. yt-lite runs the same turns in its own loop (src/main.rs),
//! with the PostgreSQL transactions it settles besides.
//!
//! An application spawns `run` on its tokio runtime; `run_until` stops between two turns once
//! its flag is set - never in the middle of an apply (review of 2026-11-13: an aborted task
//! could leave the store's transaction open). `Status` says where the link is; `say` gets
//! each change of it in words, once.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::core::relay::{self, Outcome};
use crate::core::store::{Node, ABOVE};
use crate::core::{calls, signin};

/// Where a light node's link is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
	/// the cluster above let it in
	pub granted: bool,
	/// the az it is taken to be in, as the node above said: its own (`Some("")` in a
	/// cluster with no azs)
	pub az: Option<String>,
	/// its calls are TLS: the cluster above countersigned its CA
	pub tls: bool,
	/// the last turn down found nothing new: what the cluster had is applied here
	pub caught_up: bool,
	/// local batches the cluster above has not passed yet
	pub waiting: usize,
	/// what is wrong now, if anything
	pub problem: Option<String>,
	/// turns run since it started
	pub turns: u64,
}

/// A relay turn's failure in words; None for a turn that is fine.
pub fn failed(o: &Outcome) -> Option<String> {
	match o {
		Outcome::NoTarget | Outcome::Idle | Outcome::Moved | Outcome::Stalled => None,
		Outcome::AskFailed(e) => Some(t!("light-ask-failed", error = e)),
		Outcome::FetchFailed(e) => Some(t!("light-fetch-failed", error = e)),
		Outcome::ApplyFailed(e) => Some(t!("light-apply-failed", error = e)),
		Outcome::CollectFailed(e) => Some(t!("light-collect-failed", error = e)),
		Outcome::Refused(e) => Some(t!("light-batch-refused", error = e)),
		Outcome::DeliverFailed(e) => Some(t!("light-deliver-failed", error = e)),
	}
}

/// Each problem said once, and that it is gone once.
#[derive(Default)]
struct Said(HashMap<&'static str, String>);

impl Said {
	fn say(&mut self, say: &(dyn Fn(String) + Send + Sync), key: &'static str, text: Option<String>) {
		match text {
			Some(t) if self.0.get(key) != Some(&t) => {
				say(t.clone());
				self.0.insert(key, t);
			}
			Some(_) => {}
			None => {
				if self.0.remove(key).is_some() {
					say(t!("light-fine-again", what = key));
				}
			}
		}
	}
	fn now(&self) -> Option<String> {
		self.0.values().next().cloned()
	}
}

/// The link's turns, until the task is dropped.
pub async fn run(node: Arc<Node>, status: Arc<Mutex<Status>>, say: impl Fn(String) + Send + Sync + 'static) {
	run_until(node, status, say, Arc::new(AtomicBool::new(false))).await
}

/// The link's turns, until `stop` is set: it is looked at between two turns.
pub async fn run_until(node: Arc<Node>, status: Arc<Mutex<Status>>, say: impl Fn(String) + Send + Sync + 'static,
	stop: Arc<AtomicBool>) {
	let mut said = Said::default();
	let mut granted = false;
	let mut az_said: Option<String> = None;
	let mut signed_at: Option<Instant> = None;
	let mut signed_turn = None;
	let mut turns = 0u64;
	loop {
		tokio::time::sleep(Duration::from_secs(1)).await;
		if stop.load(Ordering::Relaxed) {
			return;
		}
		let turn = calls::turn(&node).await;
		if !granted || signed_turn != Some(turn) || signed_at.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
			signed_at = Some(Instant::now());
			signed_turn = Some(turn);
			let r = signin::sign_in(&node, ABOVE).await;
			#[cfg(feature = "tls")]
			if let Some(n) = node.tls.as_ref().and_then(|t| t.news()) {
				match n {
					Ok(text) => {
						say(t!("light-tls-news", text = text));
						said.say(&say, "link TLS", None);
					}
					Err(e) => said.say(&say, "link TLS", Some(t!("light-tls-problem", error = e))),
				}
			}
			match r {
				Ok(g) if g.granted => {
					if !granted || az_said.as_deref() != Some(g.az.as_str()) {
						say(if g.az.is_empty() { t!("light-signed-in-flat") } else { t!("light-signed-in-az", az = g.az) });
						az_said = Some(g.az.clone());
					}
					granted = true;
					said.say(&say, "signin", None);
				}
				Ok(g) => {
					granted = false;
					said.say(&say, "signin", Some(calls::refused(&g.reason)));
				}
				Err(e) => {
					// the next host for one that has not let this node in (main.rs, the same)
					if !granted {
						calls::forget(&node, ABOVE).await;
					}
					granted = false;
					node.not_granted().await;
					said.say(&say, "signin", Some(t!("light-signin-failed", error = e)));
				}
			}
		}
		let mut caught_up = false;
		if granted {
			let d = relay::down(&node, ABOVE).await;
			caught_up = d == Outcome::Idle;
			said.say(&say, "down", failed(&d));
			let u = relay::up(&node, ABOVE).await;
			said.say(&say, "up", failed(&u));
		}
		turns += 1;
		let waiting = node.waiting().await;
		let az = node.told_az().await;
		#[cfg(feature = "tls")]
		let tls = node.tls.as_ref().is_some_and(|t| t.ready());
		#[cfg(not(feature = "tls"))]
		let tls = false;
		let mut s = status.lock().unwrap_or_else(|e| e.into_inner());
		*s = Status { granted, az, tls, caught_up, waiting, problem: said.now(), turns };
	}
}
