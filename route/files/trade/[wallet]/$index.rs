crate::bloom_dir_component!(
    crate::static_dir_spec(),
    vec![
        crate::writable("new"),
        crate::dir("drafts"),
        crate::dir("receipts"),
    ]
);
