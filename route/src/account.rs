//! Trusted numbered account context for explicit wallet/index routes.
use std::cell::Cell;
thread_local! { static SELECTED: Cell<u32> = const { Cell::new(0) }; }
pub fn wallet_param(ctx: &petal::Ctx) -> Result<&str, petal::DispatchResponse> {
    let wallet = petal::wallet_param(ctx)?;
    if petal::route_param(ctx, "bloom.wallet") != Some(wallet) {
        return Err(petal::error(-2, "Bloom did not select this wallet"));
    }
    let account = petal::route_param(ctx, "bloom.account")
        .ok_or_else(|| petal::error(-2, "Bloom did not select an account"))?
        .parse::<u32>()
        .map_err(|_| petal::error(-3, "invalid selected account"))?;
    SELECTED.with(|selected| selected.set(account));
    Ok(wallet)
}
pub fn number() -> u32 {
    SELECTED.with(Cell::get)
}
pub fn link(wallet: &str, path: &str) -> String {
    path.replacen(
        &format!("/{wallet}/"),
        &format!("/{wallet}/{}/", number()),
        1,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_keep_feature_subtree() {
        SELECTED.with(|n| n.set(1));
        assert_eq!(link("alice", "fund/alice/new"), "fund/alice/1/new");
        SELECTED.with(|n| n.set(0));
    }
}
