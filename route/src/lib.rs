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

mod framework;
mod host;
mod polymarket;
mod routes;

pub(crate) use crate::bloom::route::types::EntryKind;
pub(crate) use framework::*;
pub(crate) use host::bloom_petal_sdk;
#[allow(unused_imports)]
pub(crate) use host::bloom_petal_sdk::DispatchResponse;
pub(crate) use polymarket::{
    PolymarketError, Result, eip712, order, order_store, signer, trade, types,
    validate_wallet_name, wallet,
};
#[allow(unused_imports)]
pub(crate) use routes::*;

#[cfg(not(test))]
use selected_route::Route;

#[cfg(not(test))]
export!(Route);
