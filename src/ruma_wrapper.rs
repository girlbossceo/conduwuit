use {
    rocket::data::{FromDataSimple, Outcome},
    rocket::http::Status,
    rocket::response::Responder,
    rocket::Request,
    rocket::{Data, Outcome::*},
    std::ops::Deref,
    std::{
        convert::{TryFrom, TryInto},
        io::{Cursor, Read},
    },
};

const MESSAGE_LIMIT: u64 = 65535;

pub struct Ruma<T>(pub T);
impl<T: TryFrom<http::Request<Vec<u8>>>> FromDataSimple for Ruma<T> {
    type Error = ();

    fn from_data(request: &Request, data: Data) -> Outcome<Self, Self::Error> {
        let mut handle = data.open().take(MESSAGE_LIMIT);
        let mut body = Vec::new();
        handle.read_to_end(&mut body).unwrap();
        dbg!(&body);
        let mut http_request = http::Request::builder().uri(request.uri().to_string());
        for header in request.headers().iter() {
            http_request = http_request.header(header.name.as_str(), &*header.value);
        }

        let http_request = http_request.body(body).unwrap();

        match T::try_from(http_request) {
            Ok(r) => Success(Ruma(r)),
            Err(_) => Failure((Status::InternalServerError, ())),
        }
    }
}

impl<T> Deref for Ruma<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'r, T: TryInto<http::Response<Vec<u8>>>> Responder<'r> for Ruma<T> {
    fn respond_to(self, _: &Request) -> rocket::response::Result<'r> {
        match self.0.try_into() {
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
