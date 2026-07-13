#![allow(clippy::too_many_arguments)]
#![allow(dead_code, clippy::upper_case_acronyms)]

//! Local Polymarket handler petal.
//!
//! This petal owns `apps/polymarket/` directly. Public market/account reads go
//! through the v2 `bloom:http` import; staged local state goes through the
//! v2 private store import. It intentionally does not call the legacy native
//! `polymarket/` VFS handler.

pub mod account_views;
pub mod app_types;
pub mod approval;
pub mod constants;
pub mod fund_flow;
pub mod infra_parts {
    pub mod clob_l2;
    pub mod credentials;
    pub mod host_calls;
    pub mod http;
    pub mod lists;
    pub mod reconcile;
    pub mod relayer;
    pub mod store;
    pub mod util;
}
pub mod onboarding;
pub mod polymarket;
pub mod prelude;
pub mod public_reads;
pub mod relayer_actions;
pub mod relayer_config;
pub mod runtime_config;
pub mod trade_flow_parts {
    pub mod chain;
    pub mod draft;
    pub mod policy;
    pub mod posting;
    pub mod pricing;
    pub mod revalidate;
    pub mod storage;
}

#[cfg(test)]
mod app_tests;
