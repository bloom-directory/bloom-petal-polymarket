//! Trusted account selected for one wallet route invocation.
use std::cell::Cell;

thread_local! {
    static SELECTED: Cell<u32> = const { Cell::new(0) };
    static PREFIX: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub fn wallet_param(ctx: &petal::Ctx) -> Result<&str, petal::DispatchResponse> {
    let wallet = petal::wallet_param(ctx)?;
    let account = petal::route_param(ctx, "bloom.account")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    SELECTED.with(|selected| selected.set(account));
    PREFIX.with(|prefix| {
        *prefix.borrow_mut() = petal::route_param(ctx, "bloom.route_prefix").map(str::to_owned)
    });
    Ok(wallet)
}

pub fn number() -> u32 {
    SELECTED.with(Cell::get)
}

/// Convert a legacy package-relative follow-up route to the mounted path.
pub fn link(wallet: &str, old: &str) -> String {
    let Some(prefix) = PREFIX.with(|prefix| prefix.borrow().clone()) else {
        return old.to_string();
    };
    let wallet_segment = format!("/{wallet}/");
    let route = if let Some(pos) = old.find(&wallet_segment) {
        format!("{}{}", &old[..pos], &old[pos + wallet_segment.len() - 1..])
    } else {
        old.to_string()
    };
    format!("{prefix}{route}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_and_scoped_follow_up_links() {
        SELECTED.with(|selected| selected.set(1));
        assert_eq!(
            crate::infra_parts::host_calls::wallet_address_path("alice"),
            "wallets/alice/1/address.evm"
        );
        PREFIX.with(|prefix| *prefix.borrow_mut() = None);
        assert_eq!(link("alice", "fund/alice/new"), "fund/alice/new");
        PREFIX.with(|prefix| *prefix.borrow_mut() = Some("wallets/alice/1/".into()));
        assert_eq!(link("alice", "fund/alice/new"), "wallets/alice/1/fund/new");
        PREFIX.with(|prefix| *prefix.borrow_mut() = None);
        SELECTED.with(|selected| selected.set(0));
        assert_eq!(
            crate::infra_parts::host_calls::wallet_address_path("alice"),
            "wallets/alice/0/address.evm"
        );
    }
}
