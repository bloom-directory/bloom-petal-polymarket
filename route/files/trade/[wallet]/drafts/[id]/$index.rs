crate::route_file!(spec: crate::static_dir_spec(), list: {
    let mut out = crate::files(&crate::DRAFT_FILES);
    out.extend(
        crate::DRAFT_WRITABLE_FILES
            .iter()
            .map(|name| crate::writable(*name)),
    );
    out
});
