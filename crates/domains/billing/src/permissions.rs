//! Permissions of the billing context (`billing.<resource>.<action>`).
//! The identity provider grants these by name; roles are expanded at issuance.

use op_context::Permission;

pub const SETTINGS_READ: Permission = Permission::new("billing.settings.read");
pub const SETTINGS_MANAGE: Permission = Permission::new("billing.settings.manage");
pub const INVOICE_ISSUE: Permission = Permission::new("billing.invoice.issue");
pub const INVOICE_READ: Permission = Permission::new("billing.invoice.read");
pub const INVOICE_VOID: Permission = Permission::new("billing.invoice.void");
/// Held only by the `billing.record_invoice_stats` consumer's system context.
pub const STATS_RECORD: Permission = Permission::new("billing.stats.record");
// xtask:permissions

/// Every permission a human user can be granted in this context.
pub const USER_GRANTABLE: &[Permission] = &[
    SETTINGS_READ,
    SETTINGS_MANAGE,
    INVOICE_ISSUE,
    INVOICE_READ,
    INVOICE_VOID,
];
