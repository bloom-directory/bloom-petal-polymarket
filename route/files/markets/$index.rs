petal::route_file!(spec: petal::http_dir_spec(), fallible_list:
    {
        let url = crate::infra_parts::util::url_with_query(
            &format!("{}{}", crate::constants::GAMMA, "/markets"),
            &[
                ("closed", "false"),
                ("limit", &crate::constants::MARKETS_LIST_LIMIT.to_string()),
                ("order", "volumeNum"),
                ("ascending", "false"),
            ],
        );
        match crate::infra_parts::http::get_json::<Vec<crate::polymarket::Market>>(&url) {
            Ok(markets) => Ok(petal::dirs(
                markets
                    .into_iter()
                    .filter_map(|market| (!market.slug.is_empty()).then_some(market.slug))
                    .collect(),
            )),
            Err(resp) => Err(resp),
        }
    }
);
