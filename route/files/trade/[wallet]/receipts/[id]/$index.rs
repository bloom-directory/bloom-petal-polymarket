crate::bloom_dir_component!(crate::static_dir_spec(), {
    let mut out = crate::files(&crate::RECEIPT_FILES);
    out.extend(
        crate::RECEIPT_WRITABLE_FILES
            .iter()
            .map(|name| crate::writable(*name)),
    );
    out
});
