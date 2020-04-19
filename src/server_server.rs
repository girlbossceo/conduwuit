use std::convert::TryInto;

pub fn send_request<T: TryInto<http::Request<Vec<u8>>>>(
    data: &crate::Data,
    method: http::Method,
    uri: String,
    destination: String,
    request: T,
) where
    T::Error: std::fmt::Debug,
{
    let mut http_request: http::Request<_> = request.try_into().unwrap();
    let request_json = serde_json::to_value(http_request.body()).unwrap();

    let request_map = request_json.as_object_mut().unwrap();

    request_map.insert("method".to_owned(), method.to_string().into());
    request_map.insert("uri".to_owned(), uri.to_string().into());
    //TODO: request_map.insert("origin".to_owned(), data.origin().to_string().into());
    request_map.insert("destination".to_owned(), destination.to_string().into());

    ruma_signatures::sign_json(data.hostname(), data.keypair(), &mut request_json).unwrap();
    let signature = request_json["signatures"];
    data.reqwest_client().execute(http_request.into());
}
