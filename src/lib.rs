use std::convert::TryFrom;
use std::fmt::Debug;

use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::http::{
    HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::body::SdkBody;
use fastly::convert::ToBackend;
use fastly::http::request::{SendError, SendErrorCause};
use fastly::Backend;

/// Sends a [fastly::Request]. This trait can be implemented to contain your own transmission logic such as logging.
pub trait Sender: Clone + Debug + Send + Sync + 'static {
    /// Sends the [fastly::Request] and returns a result containing a [fastly::Response] or a [SendError].
    fn send(&self, request: fastly::Request) -> Result<fastly::Response, SendError>;
}

/// A [Sender] implementation that uses [fastly::Request::send] to transmit the request.
#[derive(Clone, Debug)]
pub struct DefaultSender {
    backend: Backend,
}

impl Sender for DefaultSender {
    fn send(&self, request: fastly::Request) -> Result<fastly::Response, SendError> {
        request.send(&self.backend)
    }
}

impl<T: ToBackend> From<T> for DefaultSender {
    fn from(backend: T) -> Self {
        Self {
            backend: backend.into_owned(),
        }
    }
}

/// An HTTP client for communicating with AWS services. This is what you'll insert into the [aws_config::SdkConfig].
#[derive(Debug)]
pub struct FastlyHttpClient<T: Sender> {
    sender: T,
}

impl<T: Sender> From<T> for FastlyHttpClient<T> {
    fn from(sender: T) -> Self {
        Self { sender }
    }
}

impl<T: Sender> HttpClient for FastlyHttpClient<T> {
    fn http_connector(
        &self,
        _: &HttpConnectorSettings,
        _: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(FastlyHttpConnector::from(self.sender.clone()))
    }
}

#[derive(Debug)]
pub struct FastlyHttpConnector<T: Sender> {
    sender: T,
}

impl<T: Sender> From<T> for FastlyHttpConnector<T> {
    fn from(sender: T) -> Self {
        Self { sender }
    }
}

impl<T: Sender> HttpConnector for FastlyHttpConnector<T> {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let request = Request::from(request);

        match request.send_with(&self.sender) {
            Ok(response) => HttpConnectorFuture::ready(Ok(response.into())),
            Err(error) => HttpConnectorFuture::ready(Err(error.into())),
        }
    }
}

struct Request(fastly::Request);

impl Request {
    fn send_with(self, sender: &impl Sender) -> Result<Response, Error> {
        sender.send(self.0).map(Response).map_err(Error)
    }
}

impl From<HttpRequest> for Request {
    fn from(request: HttpRequest) -> Self {
        let to_fastly_body = |body: SdkBody| {
            body.bytes()
                .map(fastly::Body::from)
                .unwrap_or(fastly::Body::new())
        };

        let request = request
            .map(to_fastly_body)
            .try_into_http02x()
            .map(fastly::Request::from)
            .unwrap();

        Self(request)
    }
}

struct Response(fastly::Response);

impl From<fastly::Response> for Response {
    fn from(response: fastly::Response) -> Self {
        Self(response)
    }
}

impl From<Response> for HttpResponse {
    fn from(response: Response) -> Self {
        let response: http::Response<fastly::Body> = response.0.into();
        let to_sdk_body = |body: fastly::Body| SdkBody::from(body.into_bytes());
        HttpResponse::try_from(response.map(to_sdk_body)).unwrap()
    }
}

struct Error(SendError);

impl From<Error> for ConnectorError {
    fn from(error: Error) -> Self {
        match error.0.root_cause() {
            SendErrorCause::BufferSize(_)
            | SendErrorCause::DnsError { .. }
            | SendErrorCause::ConnectionRefused
            | SendErrorCause::ConnectionTerminated
            | SendErrorCause::ConnectionLimitReached
            | SendErrorCause::TlsProtocolError
            | SendErrorCause::TlsAlertReceived { .. }
            | SendErrorCause::TlsConfigurationError
            | SendErrorCause::HttpIncompleteResponse
            | SendErrorCause::HttpResponseHeaderSectionTooLarge
            | SendErrorCause::HttpResponseBodyTooLarge
            | SendErrorCause::HttpProtocolError => ConnectorError::io(error.into()),
            SendErrorCause::DnsTimeout
            | SendErrorCause::ConnectionTimeout
            | SendErrorCause::HttpResponseTimeout => ConnectorError::timeout(error.into()),
            _ => ConnectorError::other(error.into(), None),
        }
    }
}

impl From<Error> for BoxError {
    fn from(val: Error) -> Self {
        Box::new(val.0)
    }
}
