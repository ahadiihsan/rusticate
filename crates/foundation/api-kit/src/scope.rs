use std::future::Future;

use uuid::Uuid;

tokio::task_local! {
    static SCOPE: RequestScope;
}

/// Facts about the in-flight HTTP request, set once by the server-kit
/// middleware and readable anywhere on the request task.
///
/// Used to fill `instance` and `request_id` in problem responses without
/// threading the request through every function.
#[derive(Debug, Clone)]
pub struct RequestScope {
    pub request_id: Uuid,
    pub path: String,
}

impl RequestScope {
    /// Runs `future` with this scope installed.
    pub async fn run<F: Future>(self, future: F) -> F::Output {
        SCOPE.scope(self, future).await
    }

    /// The scope of the current request, if called from a request task.
    pub fn current() -> Option<Self> {
        SCOPE.try_with(Clone::clone).ok()
    }
}
