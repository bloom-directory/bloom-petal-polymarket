crate::route_file!(spec: crate::static_dir_spec(), list:
    vec![
        crate::writable("new"),
        crate::dir("drafts"),
        crate::dir("receipts"),
    ]
);
