crate::bloom_dir_component!("trade/[wallet]/receipts/[id]/$index", {
    let mut out = crate::strings(&crate::RECEIPT_FILES);
    out.extend(crate::strings(&crate::RECEIPT_WRITABLE_FILES));
    out
});
