crate::bloom_dir_component!({
    let mut out = crate::files(&crate::DRAFT_FILES);
    out.extend(crate::DRAFT_WRITABLE_FILES.iter().map(|name| crate::writable(*name)));
    out
});
