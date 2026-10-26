//! The endpoints: an axum router over a [`Mesh`] (not yet: every request is answered 501).

use std::net::SocketAddr;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Router;

use crate::envelope::Reply;
use crate::mesh::{Inner, Mesh};

/// The OpenAPI document (generated from the handlers' annotations).
pub fn openapi() -> utoipa::openapi::OpenApi {
    utoipa::openapi::OpenApiBuilder::new().build()
}

pub(crate) fn metrics_text(inner: &Inner) -> String {
    let _ = inner;
    String::new()
}

async fn fallback() -> Response {
    (StatusCode::NOT_IMPLEMENTED, Body::from("not implemented")).into_response()
}

impl Mesh {
    /// The router of every endpoint.
    pub fn router(&self) -> Router {
        Router::new().fallback(fallback)
    }

    /// Serves the router on `listener` until `shutdown` resolves.
    pub async fn serve(
        &self,
        listener: tokio::net::TcpListener,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown)
            .await
    }
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        let mut r = Response::new(Body::from(self.body));
        *r.status_mut() = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        r
    }
}
