//! API request routing for the execution engine.
//!
//! Matches incoming requests against API routes using priority ordering:
//! exact matches first, then parameterized, then greedy, then $default.

use std::collections::HashMap;

use crate::storage::RouteRecord;

/// Priority ordering for route matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchPriority {
    /// Exact method + path match.
    Exact = 0,
    /// Method matches, path has parameters.
    Parameterized = 1,
    /// Greedy path parameter match (e.g., `{proxy+}`).
    Greedy = 2,
    /// The `$default` catch-all route.
    Default = 3,
}

/// Match result with extracted path parameters.
#[derive(Debug)]
struct RouteMatch<'a> {
    route: &'a RouteRecord,
    priority: MatchPriority,
    path_params: HashMap<String, String>,
}

/// Match an incoming request to the best route.
///
/// Returns the matched route and extracted path parameters, or `None` if no
/// route matches.
#[must_use]
pub fn match_route<'a, S: ::std::hash::BuildHasher>(
    routes: &'a HashMap<String, RouteRecord, S>,
    method: &http::Method,
    path: &str,
) -> Option<(&'a RouteRecord, HashMap<String, String>)> {
    let mut best_match: Option<RouteMatch<'a>> = None;

    for route in routes.values() {
        if let Some(m) = try_match(route, method, path) {
            let is_better = best_match
                .as_ref()
                .is_none_or(|current| m.priority < current.priority);
            if is_better {
                best_match = Some(m);
            }
        }
    }

    best_match.map(|m| (m.route, m.path_params))
}

/// Try to match a single route against method and path.
fn try_match<'a>(
    route: &'a RouteRecord,
    method: &http::Method,
    path: &str,
) -> Option<RouteMatch<'a>> {
    let route_key = &route.route_key;

    // Handle $default route
    if route_key == "$default" {
        return Some(RouteMatch {
            route,
            priority: MatchPriority::Default,
            path_params: HashMap::new(),
        });
    }

    // Parse route key: "METHOD /path" or "ANY /path"
    let (route_method, route_path) = route_key.split_once(' ')?;

    // Check method match (ANY matches all methods)
    if route_method != "ANY" && route_method != method.as_str() {
        return None;
    }

    // Try path matching
    match_path(route_path, path).map(|(params, priority)| RouteMatch {
        route,
        priority,
        path_params: params,
    })
}

/// Match a route path pattern against a request path.
///
/// Returns extracted parameters and match priority on success.
fn match_path(pattern: &str, path: &str) -> Option<(HashMap<String, String>, MatchPriority)> {
    let pattern_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let mut params = HashMap::new();
    let mut has_params = false;

    for (i, ps) in pattern_segments.iter().enumerate() {
        // Greedy parameter: {proxy+}
        if ps.starts_with('{') && ps.ends_with("+}") {
            let name = &ps[1..ps.len() - 2];
            let remaining: Vec<&str> = path_segments[i..].to_vec();
            params.insert(name.to_owned(), remaining.join("/"));
            return Some((params, MatchPriority::Greedy));
        }

        if i >= path_segments.len() {
            return None;
        }

        if ps.starts_with('{') && ps.ends_with('}') {
            let name = &ps[1..ps.len() - 1];
            params.insert(name.to_owned(), path_segments[i].to_owned());
            has_params = true;
        } else if *ps != path_segments[i] {
            return None;
        }
    }

    if pattern_segments.len() != path_segments.len() {
        return None;
    }

    let priority = if has_params {
        MatchPriority::Parameterized
    } else {
        MatchPriority::Exact
    };

    Some((params, priority))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_route(id: &str, route_key: &str) -> RouteRecord {
        RouteRecord {
            route_id: id.to_owned(),
            route_key: route_key.to_owned(),
            target: None,
            authorization_type: None,
            authorizer_id: None,
            authorization_scopes: Vec::new(),
            api_key_required: false,
            model_selection_expression: None,
            operation_name: None,
            request_models: HashMap::new(),
            request_parameters: HashMap::new(),
            route_response_selection_expression: None,
            route_responses: HashMap::new(),
            api_gateway_managed: false,
        }
    }

    #[test]
    fn test_should_match_exact_route() {
        let mut routes = HashMap::new();
        routes.insert("r1".to_owned(), make_route("r1", "GET /items"));
        let (route, params) =
            match_route(&routes, &http::Method::GET, "/items").expect("should match");
        assert_eq!(route.route_id, "r1");
        assert!(params.is_empty());
    }

    #[test]
    fn test_should_match_parameterized_route() {
        let mut routes = HashMap::new();
        routes.insert("r1".to_owned(), make_route("r1", "GET /items/{id}"));
        let (route, params) =
            match_route(&routes, &http::Method::GET, "/items/42").expect("should match");
        assert_eq!(route.route_id, "r1");
        assert_eq!(params.get("id").expect("has id"), "42");
    }

    #[test]
    fn test_should_prefer_exact_over_default() {
        let mut routes = HashMap::new();
        routes.insert("r1".to_owned(), make_route("r1", "GET /items"));
        routes.insert("r2".to_owned(), make_route("r2", "$default"));
        let (route, _) = match_route(&routes, &http::Method::GET, "/items").expect("should match");
        assert_eq!(route.route_id, "r1");
    }

    #[test]
    fn test_should_fallback_to_default() {
        let mut routes = HashMap::new();
        routes.insert("r1".to_owned(), make_route("r1", "$default"));
        let (route, _) =
            match_route(&routes, &http::Method::POST, "/anything").expect("should match");
        assert_eq!(route.route_id, "r1");
    }
}
