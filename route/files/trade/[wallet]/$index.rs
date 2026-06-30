petal::route_file!(spec: petal::static_dir_spec(), list:
    vec![
        petal::writable("new"),
        petal::dir("drafts"),
        petal::dir("receipts"),
    ]
);
