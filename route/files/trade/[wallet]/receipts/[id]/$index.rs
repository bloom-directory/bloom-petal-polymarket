crate::route_file!(spec: crate::static_dir_spec(), list: {
    let mut out = crate::files(&crate::RECEIPT_FILES);
    out.extend(
        crate::RECEIPT_WRITABLE_FILES
            .iter()
            .map(|name| crate::writable(*name)),
    );
    out
});
