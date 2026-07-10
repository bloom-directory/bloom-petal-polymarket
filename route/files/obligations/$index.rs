petal::route_file!(spec: petal::store_dir_spec(), list: {
    petal::files(
        crate::infra_parts::lists::vfs_wallets_or_store("onboard/")
            .into_iter()
            .map(|wallet| format!("{wallet}.json"))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
    )
});
