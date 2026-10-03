use crate::config::RouteConfig;

#[derive(Debug, Clone)]
pub struct Router {
    routes: Vec<RouteConfig>,
}

impl Router {
    pub fn new(routes: Vec<RouteConfig>) -> Self {
        // Sort routes by longest prefix match first
        let mut sorted = routes;
        sorted.sort_by_key(|b| std::cmp::Reverse(b.path_prefix.len()));
        Self { routes: sorted }
    }

    pub fn match_route<'a>(&'a self, path: &str) -> Option<&'a RouteConfig> {
        self.routes
            .iter()
            .find(|r| path.starts_with(&r.path_prefix))
    }
}
