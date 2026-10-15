use std::fmt;

/// A permission a workflow requires, named `<context>.<resource>.<action>`.
///
/// Each domain crate declares its own permissions as constants in its
/// `permissions` module; foundation code never knows domain permissions:
///
/// ```
/// use op_context::Permission;
/// pub const ORDER_PLACE: Permission = Permission::new("orders.order.place");
/// ```
///
/// There is deliberately no implicit "admin can do anything" permission: roles
/// are expanded into explicit permissions by IAM when the token is issued, so
/// every grant is auditable and four-eyes rules cannot be bypassed in code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Permission(&'static str);

impl Permission {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_its_name() {
        const ISSUE: Permission = Permission::new("orders.order.place");

        assert_eq!(ISSUE.to_string(), "orders.order.place");
        assert_eq!(ISSUE.as_str(), "orders.order.place");
    }
}
