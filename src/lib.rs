use std::convert::TryFrom;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use aws_smithy_runtime_api::client::http::{
    HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::body::SdkBody;
use fastly::convert::ToBackend;
use fastly::http::request::{PendingRequest, PollResult, SendError, SendErrorCause};
use fastly::{Backend, Body, Request, Response};
use futures::TryFutureExt;
use tokio::sync::oneshot;
use tokio::task::spawn_local;
use tokio::time::sleep;

/// An HTTP client for communicating with AWS services. This is what you'll insert into your config.
#[derive(Debug)]
pub struct FastlyHttpClient {
    backend: Backend,
}

impl<T: ToBackend> From<T> for FastlyHttpClient {
    fn from(backend: T) -> Self {
        Self {
            backend: backend.into_owned(),
        }
    }
}

impl HttpClient for FastlyHttpClient {
    fn http_connector(
        &self,
        _: &HttpConnectorSettings,
        _: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(FastlyHttpConnector::from(self.backend.clone()))
    }
}

#[derive(Debug)]
struct FastlyHttpConnector {
    backend: Backend,
}

impl From<Backend> for FastlyHttpConnector {
    fn from(backend: Backend) -> Self {
        Self { backend }
    }
}

impl HttpConnector for FastlyHttpConnector {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let request = Request::from_http_request(request);

        let future = match request.send_async(&self.backend) {
            Ok(pending_request) => ResponseFuture::from(pending_request),
            Err(error) => return HttpConnectorFuture::ready(Err(into_connector_error(error))),
        };

        let response = future
            .map_ok(into_http_response)
            .map_err(into_connector_error);

        let (tx, rx) = oneshot::channel();

        spawn_local(async move {
            let result = response.await;
            let _ = tx.send(result);
        });

        HttpConnectorFuture::new_boxed(Box::pin(async move {
            rx.await.unwrap_or_else(|e|Err(ConnectorError::io(Box::new(e))))
        }))
    }
}

trait FromHttpRequest {
    fn from_http_request(request: HttpRequest) -> Self;
}

impl FromHttpRequest for Request {
    fn from_http_request(request: HttpRequest) -> Self {
        let to_fastly_body = |body: SdkBody| body.bytes().map(Body::from).unwrap_or(Body::new());

        request
            .map(to_fastly_body)
            .try_into_http1x()
            .map(Request::from)
            .unwrap()
    }
}

fn into_http_response(response: Response) -> HttpResponse {
    let response: http::Response<Body> = response.into();
    let to_sdk_body = |body: Body| SdkBody::from(body.into_bytes());
    HttpResponse::try_from(response.map(to_sdk_body)).unwrap()
}

fn into_connector_error(error: SendError) -> ConnectorError {
    match error.root_cause() {
        SendErrorCause::DnsError { .. }
        | SendErrorCause::ConnectionRefused
        | SendErrorCause::ConnectionTerminated
        | SendErrorCause::ConnectionLimitReached
        | SendErrorCause::TlsProtocolError
        | SendErrorCause::TlsAlertReceived { .. }
        | SendErrorCause::TlsConfigurationError
        | SendErrorCause::HttpIncompleteResponse
        | SendErrorCause::HttpResponseHeaderSectionTooLarge
        | SendErrorCause::HttpResponseBodyTooLarge
        | SendErrorCause::HttpProtocolError => ConnectorError::io(Box::new(error)),
        SendErrorCause::DnsTimeout
        | SendErrorCause::ConnectionTimeout
        | SendErrorCause::HttpResponseTimeout => ConnectorError::timeout(Box::new(error)),
        _ => ConnectorError::other(Box::new(error), None),
    }
}

struct ResponseFuture {
    pending_request: Option<PendingRequest>,
}

impl From<PendingRequest> for ResponseFuture {
    fn from(pending_request: PendingRequest) -> Self {
        Self {
            pending_request: Some(pending_request),
        }
    }
}

impl Future for ResponseFuture {
    type Output = Result<Response, SendError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pending_request = self.pending_request.take().unwrap();
        match pending_request.poll() {
            PollResult::Done(result) => Poll::Ready(result),
            PollResult::Pending(pending_request) => {
                self.pending_request = Some(pending_request);

                let waker = cx.waker().clone();
                let duration = Duration::from_millis(5);

                tokio::spawn(async move {
                    sleep(duration).await;
                    waker.wake();
                });

                Poll::Pending
            }
        }
    }
}
