use std::collections::BTreeMap;

use http::{HeaderName, HeaderValue, StatusCode};
use thiserror::Error;

use crate::model::{CatalogEntry, RequestContext};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("hop must be between 1 and {max}")]
    InvalidHop { max: u8 },
    #[error("route has an invalid redirect status: {0}")]
    InvalidStatus(u16),
    #[error("route produced an invalid header name: {0}")]
    InvalidHeaderName(String),
    #[error("route produced an invalid header value for {0}")]
    InvalidHeaderValue(String),
    #[error("route contains an unknown template variable: {0}")]
    UnknownVariable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectResponse {
    pub status: StatusCode,
    pub location: String,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub final_hop: bool,
}

pub fn build_redirect(
    route: &CatalogEntry,
    hop: u8,
    context: &RequestContext,
) -> Result<RedirectResponse, EngineError> {
    if hop == 0 || hop > route.hops {
        return Err(EngineError::InvalidHop { max: route.hops });
    }

    let status = redirect_status(route.status).ok_or(EngineError::InvalidStatus(route.status))?;
    let final_hop = hop == route.hops;
    let location = if final_hop {
        render_template(&route.target, context)?
    } else {
        format!(
            "{}/r/{}/{}?{}",
            context.public_url.trim_end_matches('/'),
            route.id,
            hop + 1,
            context.query
        )
    };

    let mut headers = Vec::with_capacity(route.headers.len());
    for (name, value) in &route.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| EngineError::InvalidHeaderName(name.clone()))?;
        let value = render_template(value, context)?;
        let value = HeaderValue::from_str(&value)
            .map_err(|_| EngineError::InvalidHeaderValue(name.to_string()))?;
        headers.push((name, value));
    }

    Ok(RedirectResponse {
        status,
        location,
        headers,
        final_hop,
    })
}

/// Returns one of the redirect statuses Waybend supports consistently.
pub fn redirect_status(status: u16) -> Option<StatusCode> {
    matches!(status, 301 | 302 | 303 | 307 | 308)
        .then(|| StatusCode::from_u16(status).expect("known redirect status"))
}

pub fn render_template(template: &str, context: &RequestContext) -> Result<String, EngineError> {
    let values = BTreeMap::from([
        ("client_ip", context.client_ip.as_str()),
        ("referer_host", context.referer_host.as_str()),
        ("token", context.token.as_str()),
        ("public_url", context.public_url.as_str()),
    ]);
    let mut output = String::with_capacity(template.len() + 32);
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let variable_start = start + 2;
        let Some(relative_end) = rest[variable_start..].find("}}") else {
            return Err(EngineError::UnknownVariable(
                rest[variable_start..].to_string(),
            ));
        };
        let end = variable_start + relative_end;
        let variable = rest[variable_start..end].trim();
        let value = values
            .get(variable)
            .ok_or_else(|| EngineError::UnknownVariable(variable.to_string()))?;
        output.push_str(value);
        rest = &rest[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RequestContext {
        RequestContext {
            client_ip: "203.0.113.9".into(),
            referer_host: "probe.example".into(),
            token: "case-7".into(),
            public_url: "https://bend.example/".into(),
            query: "token=case-7".into(),
        }
    }

    fn route() -> CatalogEntry {
        CatalogEntry {
            id: "aws-role".into(),
            title: "AWS role".into(),
            category: "cloud".into(),
            description: String::new(),
            tags: vec![],
            target: "http://169.254.169.254/latest/{{token}}".into(),
            status: 302,
            hops: 3,
            headers: BTreeMap::from([("X-Origin".into(), "{{client_ip}}".into())]),
            source: "test".into(),
        }
    }

    #[test]
    fn builds_intermediate_and_final_hops_without_normalizing_target() {
        let middle = build_redirect(&route(), 1, &context()).unwrap();
        assert_eq!(
            middle.location,
            "https://bend.example/r/aws-role/2?token=case-7"
        );
        assert!(!middle.final_hop);

        let final_response = build_redirect(&route(), 3, &context()).unwrap();
        assert_eq!(
            final_response.location,
            "http://169.254.169.254/latest/case-7"
        );
        assert!(final_response.final_hop);
        assert_eq!(final_response.headers[0].1, "203.0.113.9");
    }

    #[test]
    fn preserves_non_http_scheme_and_encoded_bytes() {
        let mut value = route();
        value.hops = 1;
        value.target = "Gopher://127.0.0.1:6379/_INFO%0D%0APING".into();
        assert_eq!(
            build_redirect(&value, 1, &context()).unwrap().location,
            value.target
        );
    }

    #[test]
    fn rejects_unknown_variables_and_invalid_hops() {
        let mut value = route();
        value.target = "http://{{missing}}".into();
        assert!(matches!(
            build_redirect(&value, 3, &context()),
            Err(EngineError::UnknownVariable(name)) if name == "missing"
        ));
        assert_eq!(
            build_redirect(&route(), 0, &context()).unwrap_err(),
            EngineError::InvalidHop { max: 3 }
        );
    }
}
