petal::route_file!(spec: petal::static_dir_spec(), list:
    petal::dir_names(&[
        "markets",
        "search",
        "positions",
        "onboard",
        "account",
        "trade",
        "fund",
        "meta",
        "builder-keys",
        "settings",
        "redeem",
        "revoke-approvals",
        "withdraw",
        "obligations",
    ])
);
