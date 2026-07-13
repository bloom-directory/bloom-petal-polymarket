petal::route_file!(spec: petal::static_dir_spec(), list: vec![
    petal::writable("enso-api-key"),
    petal::writable("relayer.json"),
]);
