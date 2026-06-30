#![allow(clippy::too_many_arguments)]
#![allow(dead_code)]

//! Local Polymarket handler petal.
//!
//! This petal owns `apps/polymarket/` directly. Public market/account reads go
//! through the v2 `bloom:http` import; staged local state goes through the
//! v2 private store import. It intentionally does not call the legacy native
//! `polymarket/` VFS handler.

wit_bindgen::generate!({
    path: "wit",
    world: "route-file",
    generate_all
});

mod selected_route {
    include!(env!("BLOOM_ROUTE_RS"));
}

mod app_types;
mod constants;
mod framework;
mod fund_flow;
mod host;
mod infra_parts {
    pub(crate) mod clob_l2;
    pub(crate) mod credentials;
    pub(crate) mod host_calls;
    pub(crate) mod http;
    pub(crate) mod lists;
    pub(crate) mod reconcile;
    pub(crate) mod relayer;
    pub(crate) mod render;
    pub(crate) mod store;
    pub(crate) mod util;
}
mod onboarding;
mod polymarket;
mod public_reads;
mod trade_flow_parts {
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

pub(crate) use crate::bloom::route::types::EntryKind;
pub(crate) use app_types::*;
pub(crate) use constants::*;
pub(crate) use framework::*;
#[allow(unused_imports)]
pub(crate) use fund_flow::*;
pub(crate) use host::bloom_petal_sdk;
#[allow(unused_imports)]
pub(crate) use host::bloom_petal_sdk::DispatchResponse;
pub(crate) use infra_parts::clob_l2::*;
pub(crate) use infra_parts::credentials::*;
pub(crate) use infra_parts::host_calls::*;
pub(crate) use infra_parts::http::*;
pub(crate) use infra_parts::lists::*;
pub(crate) use infra_parts::reconcile::*;
pub(crate) use infra_parts::relayer::*;
pub(crate) use infra_parts::render::*;
pub(crate) use infra_parts::store::*;
pub(crate) use infra_parts::util::*;
pub(crate) use onboarding::*;
pub(crate) use polymarket::{
    PolymarketError, Result, eip712, order, order_store, signer, trade, types,
    validate_wallet_name, wallet,
};
#[allow(unused_imports)]
pub(crate) use public_reads::*;
pub(crate) use trade_flow_parts::chain::*;
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::draft::*;
pub(crate) use trade_flow_parts::policy::*;
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::posting::*;
pub(crate) use trade_flow_parts::pricing::*;
pub(crate) use trade_flow_parts::revalidate::*;
pub(crate) use trade_flow_parts::storage::*;

#[cfg(not(test))]
use selected_route::Route;

#[cfg(not(test))]
export!(Route);
