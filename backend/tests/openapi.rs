use axum::Router;
use serde_json::Value;
use utoipa::OpenApi;

#[test]
fn openapi_spec_contains_all_endpoints_and_hides_agent_api_key() {
    let spec: Value = serde_json::to_value(backend::api::docs::ApiDoc::openapi())
        .expect("specyfikacja OpenAPI musi się serializować do JSON");

    let version = spec["openapi"]
        .as_str()
        .expect("brak pola openapi w specyfikacji");
    assert!(
        version.starts_with("3."),
        "nieoczekiwana wersja OpenAPI: {version}"
    );

    let paths = spec["paths"]
        .as_object()
        .expect("brak obiektu paths w specyfikacji");

    let expected_operations = [
        ("/health", "get"),
        ("/api/v1/matches", "get"),
        ("/api/v1/matches/{id}", "get"),
        ("/api/v1/agents", "get"),
        ("/api/v1/agents", "post"),
        ("/api/v1/agents/leaderboard", "get"),
        ("/api/v1/agents/{id}/bets", "get"),
        ("/api/v1/predictions", "post"),
        ("/api/v1/ingest/{sport}", "post"),
        ("/api/v1/simulation/settle", "post"),
        ("/api/v1/simulation/backtest", "post"),
    ];

    for (path, method) in expected_operations {
        let path_item = paths
            .get(path)
            .unwrap_or_else(|| panic!("brak ścieżki {path} w specyfikacji OpenAPI"));
        assert!(
            path_item.get(method).is_some(),
            "brak operacji {method} dla ścieżki {path}"
        );
    }

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("brak components.schemas w specyfikacji");

    let expected_schemas = [
        "ErrorResponse",
        "HealthResponse",
        "MatchWithDetailsDto",
        "MatchDetails",
        "BasketballTeamStats",
        "Agent",
        "CreateAgentDto",
        "AgentCreatedResponse",
        "LeaderboardEntryDto",
        "Bet",
        "Prediction",
        "CreatePredictionDto",
        "IngestStartedResponse",
        "SettleResponse",
        "BacktestParams",
        "BacktestPlan",
    ];

    for name in expected_schemas {
        assert!(
            schemas.contains_key(name),
            "brak schematu {name} w components.schemas"
        );
    }

    let agent_props = schemas["Agent"]["properties"]
        .as_object()
        .expect("brak properties w schemacie Agent");
    assert!(
        !agent_props.contains_key("api_key"),
        "pole api_key nie może być ujawniane w schemacie Agent"
    );
}

#[test]
fn swagger_ui_merges_into_axum_router() {
    let _: Router<()> = Router::new().merge(backend::api::docs::swagger_ui());
}
