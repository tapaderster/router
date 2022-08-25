#![allow(missing_docs)] // FIXME

use futures::future::ready;
use futures::stream::once;
use futures::stream::BoxStream;
use futures::stream::StreamExt;
use http::header::HeaderName;
use http::method::Method;
use http::HeaderValue;
use http::StatusCode;
use http::Uri;
use multimap::MultiMap;
use serde_json_bytes::ByteString;
use serde_json_bytes::Map as JsonMap;
use serde_json_bytes::Value;
use static_assertions::assert_impl_all;
use tower::BoxError;

use crate::error::Error;
use crate::graphql;
use crate::http_ext::header_map;
use crate::http_ext::IntoHeaderName;
use crate::http_ext::IntoHeaderValue;
use crate::json_ext::Path;
use crate::Context;

pub type BoxService = tower::util::BoxService<Request, Response, BoxError>;
pub type BoxCloneService = tower::util::BoxCloneService<Request, Response, BoxError>;
pub type ServiceResult = Result<Response, BoxError>;

assert_impl_all!(Request: Send);
/// Represents the router processing step of the processing pipeline.
///
/// This consists of the parsed graphql Request, HTTP headers and contextual data for extensions.
#[non_exhaustive]
pub struct Request {
    /// Original request to the Router.
    pub originating_request: http::Request<graphql::Request>,

    /// Context for extension
    pub context: Context,
}

impl From<http::Request<graphql::Request>> for Request {
    fn from(originating_request: http::Request<graphql::Request>) -> Self {
        Self {
            originating_request,
            context: Context::new(),
        }
    }
}

#[buildstructor::buildstructor]
impl Request {
    /// This is the constructor (or builder) to use when constructing a real Request.
    ///
    /// Required parameters are required in non-testing code to create a Request.
    #[allow(clippy::too_many_arguments)]
    #[builder(visibility = "pub")]
    fn new(
        query: Option<String>,
        operation_name: Option<String>,
        // Skip the `Object` type alias in order to use buildstructor’s map special-casing
        variables: JsonMap<ByteString, Value>,
        extensions: JsonMap<ByteString, Value>,
        context: Context,
        headers: MultiMap<IntoHeaderName, IntoHeaderValue>,
        uri: Uri,
        method: Method,
    ) -> Result<Request, BoxError> {
        let gql_request = graphql::Request::builder()
            .and_query(query)
            .and_operation_name(operation_name)
            .variables(variables)
            .extensions(extensions)
            .build();
        let mut originating_request = http::Request::builder()
            .uri(uri)
            .method(method)
            .body(gql_request)?;
        *originating_request.headers_mut() = header_map(headers)?;
        Ok(Self {
            originating_request,
            context,
        })
    }

    /// This is the constructor (or builder) to use when constructing a "fake" Request.
    ///
    /// This does not enforce the provision of the data that is required for a fully functional
    /// Request. It's usually enough for testing, when a fully constructed Request is
    /// difficult to construct and not required for the purposes of the test.
    ///
    /// In addition, fake requests are expected to be valid, and will panic if given invalid values.
    #[builder(visibility = "pub")]
    fn fake_new(
        query: Option<String>,
        operation_name: Option<String>,
        // Skip the `Object` type alias in order to use buildstructor’s map special-casing
        variables: JsonMap<ByteString, Value>,
        extensions: JsonMap<ByteString, Value>,
        context: Option<Context>,
        mut headers: MultiMap<IntoHeaderName, IntoHeaderValue>,
        method: Option<Method>,
    ) -> Result<Request, BoxError> {
        // Avoid testing requests getting blocked by the CSRF-prevention plugin
        headers
            .entry(IntoHeaderName::HeaderName(http::header::CONTENT_TYPE))
            .or_insert(IntoHeaderValue::HeaderValue(HeaderValue::from_static(
                "application/json",
            )));
        Request::new(
            query,
            operation_name,
            variables,
            extensions,
            context.unwrap_or_default(),
            headers,
            Uri::from_static("http://default"),
            method.unwrap_or(Method::GET),
        )
    }

    /// Create a request with an example query, for tests
    #[builder(visibility = "pub")]
    fn canned_new(
        operation_name: Option<String>,
        // Skip the `Object` type alias in order to use buildstructor’s map special-casing
        extensions: JsonMap<ByteString, Value>,
        context: Option<Context>,
        headers: MultiMap<IntoHeaderName, IntoHeaderValue>,
    ) -> Result<Request, BoxError> {
        let query = "
            query TopProducts($first: Int) { 
                topProducts(first: $first) { 
                    upc 
                    name 
                    reviews { 
                        id 
                        product { name } 
                        author { id name } 
                    } 
                } 
            }
        ";
        let mut variables = JsonMap::new();
        variables.insert("first", 2_usize.into());
        Self::fake_new(
            Some(query.to_owned()),
            operation_name,
            variables,
            extensions,
            context,
            headers,
            None,
        )
    }
}

assert_impl_all!(Response: Send);
#[non_exhaustive]
pub struct Response {
    pub response: http::Response<BoxStream<'static, graphql::Response>>,
    pub context: Context,
}

#[buildstructor::buildstructor]
impl Response {
    /// This is the constructor (or builder) to use when constructing a real Response..
    ///
    /// Required parameters are required in non-testing code to create a Response..
    #[allow(clippy::too_many_arguments)]
    #[builder(visibility = "pub")]
    fn new(
        data: Option<Value>,
        path: Option<Path>,
        errors: Vec<Error>,
        // Skip the `Object` type alias in order to use buildstructor’s map special-casing
        extensions: JsonMap<ByteString, Value>,
        status_code: Option<StatusCode>,
        headers: MultiMap<IntoHeaderName, IntoHeaderValue>,
        context: Context,
    ) -> Result<Self, BoxError> {
        // Build a response
        let b = graphql::Response::builder()
            .and_path(path)
            .errors(errors)
            .extensions(extensions);
        let res = match data {
            Some(data) => b.data(data).build(),
            None => b.build(),
        };

        // Build an http Response
        let mut builder = http::Response::builder().status(status_code.unwrap_or(StatusCode::OK));
        for (key, values) in headers {
            let header_name: HeaderName = key.try_into()?;
            for value in values {
                let header_value: HeaderValue = value.try_into()?;
                builder = builder.header(header_name.clone(), header_value);
            }
        }

        let response = builder.body(once(ready(res)).boxed())?;

        Ok(Self { response, context })
    }

    /// This is the constructor (or builder) to use when constructing a "fake" Response.
    ///
    /// This does not enforce the provision of the data that is required for a fully functional
    /// Response. It's usually enough for testing, when a fully constructed Response is
    /// difficult to construct and not required for the purposes of the test.
    ///
    /// In addition, fake responses are expected to be valid, and will panic if given invalid values.
    #[allow(clippy::too_many_arguments)]
    #[builder(visibility = "pub")]
    fn fake_new(
        data: Option<Value>,
        path: Option<Path>,
        errors: Vec<Error>,
        // Skip the `Object` type alias in order to use buildstructor’s map special-casing
        extensions: JsonMap<ByteString, Value>,
        status_code: Option<StatusCode>,
        headers: MultiMap<IntoHeaderName, IntoHeaderValue>,
        context: Option<Context>,
    ) -> Result<Self, BoxError> {
        Response::new(
            data,
            path,
            errors,
            extensions,
            status_code,
            headers,
            context.unwrap_or_default(),
        )
    }

    /// This is the constructor (or builder) to use when constructing a Response that represents a global error.
    /// It has no path and no response data.
    /// This is useful for things such as authentication errors.
    #[builder(visibility = "pub")]
    fn error_new(
        errors: Vec<Error>,
        status_code: Option<StatusCode>,
        headers: MultiMap<IntoHeaderName, IntoHeaderValue>,
        context: Context,
    ) -> Result<Self, BoxError> {
        Response::new(
            Default::default(),
            None,
            errors,
            Default::default(),
            status_code,
            headers,
            context,
        )
    }

    pub fn new_from_graphql_response(response: graphql::Response, context: Context) -> Self {
        Self {
            response: http::Response::new(once(ready(response)).boxed()),
            context,
        }
    }
}

impl Response {
    pub async fn next_response(&mut self) -> Option<graphql::Response> {
        self.response.body_mut().next().await
    }

    pub fn new_from_response(
        response: http::Response<BoxStream<'static, graphql::Response>>,
        context: Context,
    ) -> Self {
        Self { response, context }
    }

    pub fn map<F>(self, f: F) -> Response
    where
        F: FnOnce(BoxStream<'static, graphql::Response>) -> BoxStream<'static, graphql::Response>,
    {
        Response {
            context: self.context,
            response: self.response.map(f),
        }
    }

    pub fn map_stream(
        self,
        f: impl FnMut(graphql::Response) -> graphql::Response + Send + 'static,
    ) -> Self {
        self.map(move |stream| stream.map(f).boxed())
    }
}

#[cfg(test)]
mod test {
    use http::HeaderValue;
    use http::Method;
    use http::Uri;
    use serde_json::json;

    use super::*;
    use crate::graphql;

    #[test]
    fn supergraph_request_builder() {
        let request = Request::builder()
            .header("a", "b")
            .header("a", "c")
            .uri(Uri::from_static("http://example.com"))
            .method(Method::POST)
            .query("query { topProducts }")
            .operation_name("Default")
            .context(Context::new())
            // We need to follow up on this. How can users creat this easily?
            .extension("foo", json!({}))
            // We need to follow up on this. How can users creat this easily?
            .variable("bar", json!({}))
            .build()
            .unwrap();
        assert_eq!(
            request
                .originating_request
                .headers()
                .get_all("a")
                .into_iter()
                .collect::<Vec<_>>(),
            vec![HeaderValue::from_static("b"), HeaderValue::from_static("c")]
        );
        assert_eq!(
            request.originating_request.uri(),
            &Uri::from_static("http://example.com")
        );
        assert_eq!(
            request.originating_request.body().extensions.get("foo"),
            Some(&json!({}).into())
        );
        assert_eq!(
            request.originating_request.body().variables.get("bar"),
            Some(&json!({}).into())
        );
        assert_eq!(request.originating_request.method(), Method::POST);

        let extensions = serde_json_bytes::Value::from(json!({"foo":{}}))
            .as_object()
            .unwrap()
            .clone();

        let variables = serde_json_bytes::Value::from(json!({"bar":{}}))
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            request.originating_request.body(),
            &graphql::Request::builder()
                .variables(variables)
                .extensions(extensions)
                .operation_name("Default")
                .query("query { topProducts }")
                .build()
        );
    }

    #[tokio::test]
    async fn supergraph_response_builder() {
        let mut response = Response::builder()
            .header("a", "b")
            .header("a", "c")
            .context(Context::new())
            .extension("foo", json!({}))
            .data(json!({}))
            .build()
            .unwrap();

        assert_eq!(
            response
                .response
                .headers()
                .get_all("a")
                .into_iter()
                .collect::<Vec<_>>(),
            vec![HeaderValue::from_static("b"), HeaderValue::from_static("c")]
        );
        let extensions = serde_json_bytes::Value::from(json!({"foo":{}}))
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            response.next_response().await.unwrap(),
            graphql::Response::builder()
                .extensions(extensions)
                .data(json!({}))
                .build()
        );
    }
}