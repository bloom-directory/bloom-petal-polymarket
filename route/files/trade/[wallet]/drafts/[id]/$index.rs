crate::bloom_dir_component!("trade/[wallet]/drafts/[id]/$index", {
    let mut out = crate::strings(&crate::DRAFT_FILES);
    out.extend(crate::strings(&crate::DRAFT_WRITABLE_FILES));
    out
});
