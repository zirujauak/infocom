use actix_service::{Service, Transform};
use actix_web::{Result, dev::ServiceRequest, dev::ServiceResponse, Error};
use std::task::{Context, Poll};
use std::pin::Pin;
use log::{debug};
use futures::future::{ok, Ready};
use futures::Future;
use std::time::Instant;

pub struct Performance;

pub struct PerformanceMiddleware<S> {
    service: S
}

impl<S, B> Transform<S> for Performance
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = PerformanceMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(PerformanceMiddleware { service })
    }
}

impl<S, B> Service for PerformanceMiddleware<S>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let start = Instant::now();
        let method = req.method().to_string();
        let uri = req.uri().to_string();
        let fut = self.service.call(req);

        Box::pin(async move {
            let res = fut.await?;
            debug!("{} {}: {}ms", method, uri, start.elapsed().as_millis());
            Ok(res)
        })
    }
}