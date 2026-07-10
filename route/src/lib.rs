#![allow(clippy::too_many_arguments)]
#![allow(dead_code, clippy::upper_case_acronyms)]

//! Local Polymarket handler petal.
//!
//! This petal owns `apps/polymarket/` directly. Public market/account reads go
//! through the v2 `bloom:http` import; staged local state goes through the
//! v2 private store import. It intentionally does not call the legacy native
//! `polymarket/` VFS handler.

mod selected_route {
    include!(env!("BLOOM_ROUTE_RS"));
}

pub(crate) mod account_views;
pub(crate) mod app_types;
pub(crate) mod approval;
pub(crate) mod constants;
pub(crate) mod fund_flow;
pub(crate) mod infra_parts {
    pub(crate) mod clob_l2;
    pub(crate) mod credentials;
    pub(crate) mod host_calls;
    pub(crate) mod http;
    pub(crate) mod lists;
    pub(crate) mod reconcile;
    pub(crate) mod relayer;
    pub(crate) mod store;
    pub(crate) mod util;
}
pub(crate) mod onboarding;
pub(crate) mod polymarket;
pub(crate) mod prelude;
pub(crate) mod public_reads;
pub(crate) mod relayer_actions;
pub(crate) mod trade_flow_parts {
    pub(crate) mod chain;
    pub(crate) mod draft;
    pub(crate) mod policy;
    pub(crate) mod posting;
    pub(crate) mod pricing;
    pub(crate) mod revalidate;
    pub(crate) mod storage;
}

#[cfg(test)]
mod app_tests;

#[cfg(not(test))]
use selected_route::Route;

#[cfg(not(test))]
petal::bindings::export!(Route);
