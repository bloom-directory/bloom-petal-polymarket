petal::route_file!(spec: petal::static_dir_spec(), list: {
    let mut out = petal::files(&[
        "plan.md",
        "order.json",
        "approval.json",
        "policy_check.json",
        "quote.json",
        "review_intent.json",
        "post_attempt.json",
    ]);
    out.extend(["revalidate", "post"].iter().map(|name| petal::writable(*name)));
    out
});
