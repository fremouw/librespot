use librespot_core::authentication::Credentials;
use librespot_core::config::SessionConfig;
use librespot_core::session::Session;

#[tokio::test]
async fn test_connection() {
    let result = Session::connect(
        SessionConfig::default(),
        Credentials::with_password("test", "test"),
        None,
    )
    .await;

    match result {
        Ok(_) => panic!("Authentication succeeded despite of bad credentials."),
        Err(e) => assert_eq!(e.to_string(), "Login failed with reason: Bad credentials"),
    };
}
