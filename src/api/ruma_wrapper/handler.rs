use std::future::Future;

use axum::{
	response::IntoResponse,
	routing::{on, MethodFilter},
	Router,
};
use conduit::Result;
use http::Method;
use ruma::api::IncomingRequest;

use super::{Ruma, RumaResponse};

pub(in super::super) trait RouterExt {
	fn ruma_route<H, T>(self, handler: H) -> Self
	where
		H: RumaHandler<T>;
}

impl RouterExt for Router {
	fn ruma_route<H, T>(self, handler: H) -> Self
	where
		H: RumaHandler<T>,
	{
		handler.add_routes(self)
	}
}

pub(in super::super) trait RumaHandler<T> {
	fn add_routes(&self, router: Router) -> Router;

	fn add_route(&self, router: Router, path: &str) -> Router;
}

impl<Req, E, F, Fut> RumaHandler<Ruma<Req>> for F
where
	Req: IncomingRequest + Send + 'static,
	F: FnOnce(Ruma<Req>) -> Fut + Clone + Send + Sync + 'static,
	Fut: Future<Output = Result<Req::OutgoingResponse, E>> + Send,
	E: IntoResponse,
{
	fn add_routes(&self, router: Router) -> Router {
		Req::METADATA
			.history
			.all_paths()
			.fold(router, |router, path| self.add_route(router, path))
	}

	fn add_route(&self, router: Router, path: &str) -> Router {
		let handle = self.clone();
		let method = method_to_filter(&Req::METADATA.method);
		let action = |req| async { handle(req).await.map(RumaResponse) };
		router.route(path, on(method, action))
	}
}

const fn method_to_filter(method: &Method) -> MethodFilter {
	match *method {
		Method::DELETE => MethodFilter::DELETE,
		Method::GET => MethodFilter::GET,
		Method::HEAD => MethodFilter::HEAD,
		Method::OPTIONS => MethodFilter::OPTIONS,
		Method::PATCH => MethodFilter::PATCH,
		Method::POST => MethodFilter::POST,
		Method::PUT => MethodFilter::PUT,
		Method::TRACE => MethodFilter::TRACE,
		_ => panic!("Unsupported HTTP method"),
	}
}
