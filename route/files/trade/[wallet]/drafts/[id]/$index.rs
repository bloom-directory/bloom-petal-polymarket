crate::bloom_dir_component!(crate::static_dir_spec(), {
    let mut out = crate::files(&crate::DRAFT_FILES);
    out.extend(
        crate::DRAFT_WRITABLE_FILES
            .iter()
            .map(|name| crate::writable(*name)),
    );
    out
});
