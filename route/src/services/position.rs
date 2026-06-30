pub(crate) fn wallets() -> Vec<crate::RouteChild> {
    crate::dirs(super::vfs_wallets_or_store(""))
}

pub(crate) fn files() -> Vec<crate::RouteChild> {
    crate::files(&super::POSITION_FILES)
}

pub(crate) fn positions_json(wallet: &str) -> crate::DispatchResponse {
    super::positions_json(wallet)
}

pub(crate) fn trades_json(wallet: &str) -> crate::DispatchResponse {
    super::position_trades_json(wallet)
}

pub(crate) fn activity_json(wallet: &str) -> crate::DispatchResponse {
    super::position_activity_json(wallet)
}
