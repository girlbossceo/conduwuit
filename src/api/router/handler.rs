use std::future::Future;

use axum::{
	extract::FromRequestParts,
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

macro_rules! ruma_handler {
	( $($tx:ident),* $(,)? ) => {
		#[allow(non_snake_case)]
		impl<Req, Ret, Fut, Fun, $($tx,)*> RumaHandler<($($tx,)* Ruma<Req>,)> for Fun
		where
			Req: IncomingRequest + Send + 'static,
			Ret: IntoResponse,
			Fut: Future<Output = Result<Req::OutgoingResponse, Ret>> + Send,
			Fun: FnOnce($($tx,)* Ruma<Req>) -> Fut + Clone + Send + Sync + 'static,
			$( $tx: FromRequestParts<()> + Send + 'static, )*
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
				let action = |$($tx,)* req| async { handle($($tx,)* req).await.map(RumaResponse) };
				router.route(path, on(method, action))
			}
		}
	}
}
ruma_handler!();
ruma_handler!(T1);
ruma_handler!(T1, T2);
ruma_handler!(T1, T2, T3);
ruma_handler!(T1, T2, T3, T4);

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
