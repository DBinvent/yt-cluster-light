// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The link to the cluster above.
//!
//! Shared files - the same for the light node and for ytserv, which compiles them
//! by path: `pos`, `route`, `relay`, `signin`. They are written for a link between two
//! azs, as ytserv runs it; a light node has no az and answers with none wherever they
//! ask (`store.rs`). They reach the node only through
//! plain functions of the replaceable files beside them: `store`, `members`,
//! `calls`, `offer`. These are the light node's versions; ytserv holds its own
//! files of the same names, and is the check that the names and signatures here
//! stay the ones it gives.
pub mod pos;
pub mod relay;
pub mod route;
pub mod signin;

pub mod calls;
pub mod members;
pub mod offer;
pub mod store;
