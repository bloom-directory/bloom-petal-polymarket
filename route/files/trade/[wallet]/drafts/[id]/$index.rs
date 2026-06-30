crate::route_file!(spec: crate::static_dir_spec(), list: {
    let mut out = crate::files(&[
        "plan.md",
        "order.json",
        "policy_check.json",
        "quote.json",
        "review_intent.json",
        "post_attempt.json",
    ]);
    out.extend(["revalidate", "post"].iter().map(|name| crate::writable(*name)));
    out
});
