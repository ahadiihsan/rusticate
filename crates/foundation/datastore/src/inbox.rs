use uuid::Uuid;

use crate::{DataStoreError, TenantTx};

impl TenantTx {
    /// Records that `consumer` handled `event_id` (MSG-004).
    ///
    /// Returns `false` when the event was already recorded: the caller must
    /// then skip its effects (redelivery). Call it first in the handler's
    /// transaction so the record and the effects commit or roll back together.
    pub async fn record_inbox(
        &mut self,
        consumer: &str,
        event_id: Uuid,
    ) -> Result<bool, DataStoreError> {
        let result = sqlx::query!(
            r#"
            INSERT INTO platform.inbox_events (tenant_id, consumer, event_id)
            VALUES ($1, $2, $3)
            ON CONFLICT DO NOTHING
            "#,
            self.tenant_id().as_uuid(),
            consumer,
            event_id,
        )
        .execute(self.conn())
        .await?;
        Ok(result.rows_affected() == 1)
    }
}
