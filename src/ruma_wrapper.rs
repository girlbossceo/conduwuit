use {
    rocket::data::{FromDataSimple, Outcome},
    rocket::http::Status,
    rocket::response::Responder,
    rocket::Request,
    rocket::{Data, Outcome::*},
    ruma_client_api::error::Error,
    std::fmt::Debug,
    std::ops::Deref,
    std::{
        convert::{TryFrom, TryInto},
        io::{Cursor, Read},
    },
};

const MESSAGE_LIMIT: u64 = 65535;

pub struct Ruma<T>(pub T);
impl<T: TryFrom<http::Request<Vec<u8>>>> FromDataSimple for Ruma<T>
where
    T::Error: Debug,
{
    type Error = ();

    fn from_data(request: &Request, data: Data) -> Outcome<Self, Self::Error> {
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

        log::info!("{:?}", http_request);
        match T::try_from(http_request) {
            Ok(r) => Success(Ruma(r)),
            Err(e) => {
                log::error!("{:?}", e);
                Failure((Status::InternalServerError, ()))
            }
        }
    }
}

impl<T> Deref for Ruma<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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
                response.ok()
            }
            Err(_) => Err(Status::InternalServerError),
        }
    }
}
