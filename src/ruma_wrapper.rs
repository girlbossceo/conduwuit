use {
    rocket::data::{FromDataSimple, Outcome},
    rocket::http::Status,
    rocket::response::Responder,
    rocket::Outcome::*,
    rocket::Request,
    rocket::State,
    ruma_api::{
        error::{FromHttpRequestError, FromHttpResponseError},
        Endpoint, Outgoing,
    },
    ruma_client_api::error::Error,
    std::ops::Deref,
    std::{
        convert::{TryFrom, TryInto},
        fmt,
        io::{Cursor, Read},
    },
};

const MESSAGE_LIMIT: u64 = 65535;

/// This struct converts rocket requests into ruma structs by converting them into http requests
/// first.
pub struct Ruma<T: Outgoing> {
    body: T::Incoming,
    headers: http::HeaderMap<http::header::HeaderValue>,
}

impl<T: Endpoint> FromDataSimple for Ruma<T>
where
    // We need to duplicate Endpoint's where clauses because the compiler is not smart enough yet.
    // See https://github.com/rust-lang/rust/issues/54149
    <T as Outgoing>::Incoming: TryFrom<http::Request<Vec<u8>>, Error = FromHttpRequestError>,
    <T::Response as Outgoing>::Incoming: TryFrom<
        http::Response<Vec<u8>>,
        Error = FromHttpResponseError<<T as Endpoint>::ResponseError>,
    >,
{
    type Error = ();

    fn from_data(request: &Request, data: rocket::Data) -> Outcome<Self, Self::Error> {
        let mut http_request = http::Request::builder()
            .uri(request.uri().to_string())
            .method(&*request.method().to_string());
        for header in request.headers().iter() {
            http_request = http_request.header(header.name.as_str(), &*header.value);
        }

        let mut handle = data.open().take(MESSAGE_LIMIT);
        let mut body = Vec::new();
        handle.read_to_end(&mut body).unwrap();

        let http_request = http_request.body(body).unwrap();
        let headers = http_request.headers().clone();

        log::info!("{:?}", http_request);
        match T::Incoming::try_from(http_request) {
            Ok(t) => {
                if T::METADATA.requires_authentication {
                    let data = request.guard::<State<crate::Data>>();
                    // TODO: auth
                }
                Success(Ruma { body: t, headers })
            }
            Err(e) => {
                log::error!("{:?}", e);
                Failure((Status::InternalServerError, ()))
            }
        }
    }
}

impl<T: Outgoing> Deref for Ruma<T> {
    type Target = T::Incoming;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

impl<T: Outgoing> fmt::Debug for Ruma<T>
where
    T::Incoming: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ruma")
            .field("body", &self.body)
            .field("headers", &self.headers)
            .finish()
    }
}

/// This struct converts ruma responses into rocket http responses.
pub struct MatrixResult<T>(pub std::result::Result<T, Error>);
impl<T: TryInto<http::Response<Vec<u8>>>> TryInto<http::Response<Vec<u8>>> for MatrixResult<T> {
    type Error = T::Error;

    fn try_into(self) -> Result<http::Response<Vec<u8>>, T::Error> {
        match self.0 {
            Ok(t) => t.try_into(),
            Err(e) => Ok(e.into()),
        }
    }
}

impl<'r, T: TryInto<http::Response<Vec<u8>>>> Responder<'r> for MatrixResult<T> {
    fn respond_to(self, _: &Request) -> rocket::response::Result<'r> {
        let http_response: Result<http::Response<_>, _> = self.try_into();
        match http_response {
            Ok(http_response) => {
                let mut response = rocket::response::Response::build();
                response.sized_body(Cursor::new(http_response.body().clone()));

                for header in http_response.headers() {
                    response
                        .raw_header(header.0.to_string(), header.1.to_str().unwrap().to_owned());
                }

                response.raw_header("Access-Control-Allow-Origin", "*");
                response.raw_header(
                    "Access-Control-Allow-Methods",
                    "GET, POST, PUT, DELETE, OPTIONS",
                );
                response.raw_header(
                    "Access-Control-Allow-Headers",
                    "Origin, X-Requested-With, Content-Type, Accept, Authorization",
                );
                response.ok()
            }
            Err(_) => Err(Status::InternalServerError),
        }
    }
}
