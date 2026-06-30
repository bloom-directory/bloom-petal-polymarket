pub(crate) fn wallets() -> Vec<crate::RouteChild> {
    crate::dirs(super::vfs_wallets_or_store("creds/"))
}

pub(crate) fn files() -> Vec<crate::RouteChild> {
    crate::files(&super::ACCOUNT_FILES)
}

pub(crate) fn portfolio_json(wallet: &str) -> crate::DispatchResponse {
    super::account_portfolio_json(wallet)
}

pub(crate) fn orders_json(wallet: &str) -> crate::DispatchResponse {
    super::account_orders_json(wallet)
}
