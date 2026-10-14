use billing::BillingError;
use billing::configure_billing::{ConfigureBillingCmd, Expectation, configure_billing};
use billing::get_billing_settings::get_billing_settings;
use billing::permissions::SETTINGS_READ;
use billing_engine::money::{Bps, Currency};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::ctx;

use crate::support;

fn cmd(currency: Currency, bps: u16, expectation: Expectation) -> ConfigureBillingCmd {
    ConfigureBillingCmd {
        currency,
        tax_rate: Bps::new(bps).unwrap(),
        expectation,
    }
}

#[sqlx::test(migrations = false)]
async fn create_then_update_with_the_current_version(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());

    let created = configure_billing(
        &admin,
        &db.app,
        cmd(Currency::Idr, 1_100, Expectation::Create),
    )
    .await
    .unwrap();
    let updated = configure_billing(
        &admin,
        &db.app,
        cmd(
            Currency::Usd,
            0,
            Expectation::Update {
                version: created.version,
            },
        ),
    )
    .await
    .unwrap();

    assert_eq!(created.version, 1);
    assert_eq!(updated.version, 2);
    let current = get_billing_settings(&admin, &db.app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.currency, Currency::Usd);
}

#[sqlx::test(migrations = false)]
async fn stale_versions_and_duplicate_creates_are_rejected(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;

    let duplicate =
        configure_billing(&admin, &db.app, cmd(Currency::Idr, 0, Expectation::Create)).await;
    let stale = configure_billing(
        &admin,
        &db.app,
        cmd(Currency::Idr, 0, Expectation::Update { version: 99 }),
    )
    .await;

    assert!(matches!(duplicate, Err(BillingError::SettingsAlreadyExist)));
    assert!(matches!(stale, Err(BillingError::StaleVersion)));
}

#[sqlx::test(migrations = false)]
async fn settings_are_per_tenant_and_permission_gated(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let tenant_a = ctx::tenant();
    support::configure(&db, &support::admin(tenant_a)).await;

    let other_tenant = get_billing_settings(&support::admin(ctx::tenant()), &db.app)
        .await
        .unwrap();
    let reader_without_manage = ctx::user(tenant_a, &[SETTINGS_READ]);
    let forbidden = configure_billing(
        &reader_without_manage,
        &db.app,
        cmd(Currency::Idr, 0, Expectation::Update { version: 1 }),
    )
    .await;

    assert_eq!(other_tenant, None);
    assert!(matches!(forbidden, Err(BillingError::Forbidden(_))));
}
