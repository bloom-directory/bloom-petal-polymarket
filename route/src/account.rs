//! Trusted numbered account context, passed explicitly into domain operations.
pub fn number(ctx: &petal::Ctx) -> Result<u32, petal::DispatchResponse> {
    if petal::route_param(ctx, "bloom.wallet") != petal::route_param(ctx, "wallet") {
        return Err(petal::error(-2, "Bloom did not select this wallet"));
    }
    petal::route_param(ctx, "bloom.account")
        .ok_or_else(|| petal::error(-2, "Bloom did not select an account"))?
        .parse::<u32>()
        .map_err(|_| petal::error(-3, "invalid selected account"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Route;
    impl petal::RouteIdentity for Route {
        const PATH: &'static str = "trade/[wallet]/[index]/new";
        const CANONICAL_PATH: &'static str = "trade/[wallet]/[index]/new";
        const PARAMS: &'static [(&'static str, usize)] = &[("wallet", 1), ("index", 2)];
    }
    fn context(account: Option<&str>) -> petal::Ctx {
        let mut params = vec![
            ("wallet".into(), "alice".into()),
            ("bloom.wallet".into(), "alice".into()),
        ];
        if let Some(account) = account {
            params.push(("bloom.account".into(), account.into()));
        }
        petal::Ctx::bind::<Route>(petal::RawCtx {
            petal_root: "polymarket".into(),
            package_hash: "test".into(),
            path: "trade/alice/1/new".into(),
            params,
            actor: None,
        })
    }
    #[test]
    fn selected_accounts_are_independent_and_missing_context_fails() {
        let zero = context(Some("0"));
        let one = context(Some("1"));
        assert_eq!(number(&zero), Ok(0));
        assert_eq!(number(&one), Ok(1));
        assert_eq!(number(&zero), Ok(0));
        assert!(number(&context(None)).is_err());
        assert!(number(&context(Some("bad"))).is_err());
    }
}
