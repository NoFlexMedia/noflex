// SPDX-License-Identifier: AGPL-3.0-only

use noflex_graphql::{SchemaContext, build_schema};

#[tokio::test]
async fn rejects_queries_over_depth_limit() {
    let schema = build_schema(SchemaContext::default());
    let mut selection = "name".to_string();
    for _ in 0..13 {
        selection = format!("ofType {{ {selection} }}");
    }
    let response = schema
        .execute(format!(
            "{{ __type(name: \"Query\") {{ fields {{ type {{ {selection} }} }} }} }}"
        ))
        .await;
    assert!(
        response
            .errors
            .iter()
            .any(|error| error.message.contains("nested too deep")),
        "unexpected errors: {:?}",
        response.errors
    );
}

#[tokio::test]
async fn rejects_queries_over_complexity_limit() {
    let schema = build_schema(SchemaContext::default());
    let fields = (0..2001)
        .map(|index| format!("field{index}: serverInfo {{ name }}"))
        .collect::<Vec<_>>()
        .join(" ");
    let response = schema.execute(format!("{{ {fields} }}")).await;
    assert!(
        response
            .errors
            .iter()
            .any(|error| error.message.contains("too complex")),
        "unexpected errors: {:?}",
        response.errors
    );
}
