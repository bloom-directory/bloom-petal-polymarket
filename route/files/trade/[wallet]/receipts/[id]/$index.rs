crate::bloom_dir_component!({
    let mut out = crate::files(&crate::RECEIPT_FILES);
    out.extend(crate::RECEIPT_WRITABLE_FILES.iter().map(|name| crate::writable(*name)));
    out
});
