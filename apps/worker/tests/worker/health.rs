use server_kit::ReadinessCheck as _;
use worker::health::NatsReadiness;

#[tokio::test]
async fn nats_readiness_reflects_the_connection() {
    let client = async_nats::connect("nats://127.0.0.1:4222")
        .await
        .expect("NATS must be running (cargo rusticate db up)");
    let check = NatsReadiness(client.clone());

    assert_eq!(check.name(), "nats");
    assert_eq!(check.check().await, Ok(()));
    client.drain().await.unwrap();
}
