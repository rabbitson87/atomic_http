use std::fmt;

use http::Method;

/// Error returned when inserting a route fails.
#[derive(Debug)]
pub struct InsertError(matchit::InsertError);

impl fmt::Display for InsertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for InsertError {}

/// Zero-copy path parameters extracted from a matched route.
///
/// Parameter keys (e.g. `"id"`) reference the router's internal trie,
/// and values (e.g. `"42"`) reference the original request path — no
/// allocations are performed during matching.
pub struct Params<'k, 'v>(matchit::Params<'k, 'v>);

impl<'k, 'v> Params<'k, 'v> {
    /// Get a parameter value by name.
    ///
    /// ```text
    /// route: "/users/{id}"
    /// path:  "/users/42"
    /// params.get("id") => Some("42")
    /// ```
    pub fn get(&self, key: &str) -> Option<&'v str> {
        self.0.get(key)
    }

    /// Iterate over all `(key, value)` pairs.
    pub fn iter(&self) -> matchit::ParamsIter<'_, 'k, 'v> {
        self.0.iter()
    }

    /// Returns `true` if there are no parameters.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of parameters.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// A matched route containing the stored value and extracted parameters.
pub struct Match<'k, 'v, V> {
    pub value: &'v V,
    pub params: Params<'k, 'v>,
}

/// A method-aware HTTP router backed by a radix trie.
///
/// `V` is the value type stored per route — typically an enum variant,
/// string tag, or handler identifier.
///
/// # Example
///
/// ```rust,no_run
/// use atomic_http::router::Router;
///
/// enum Route { Home, GetUser, CreateUser }
///
/// let router = Router::new()
///     .get("/", Route::Home)
///     .get("/users/{id}", Route::GetUser)
///     .post("/users", Route::CreateUser);
/// ```
pub struct Router<V> {
    trees: Vec<(Method, matchit::Router<V>)>,
}

impl<V> Router<V> {
    /// Create an empty router.
    pub fn new() -> Self {
        Self { trees: Vec::new() }
    }

    /// Insert a route for the given HTTP method and path pattern.
    ///
    /// Path patterns support:
    /// - Named parameters: `/users/{id}`
    /// - Catch-all: `/files/{*path}`
    pub fn insert(&mut self, method: Method, path: &str, value: V) -> Result<(), InsertError> {
        if let Some((_, tree)) = self.trees.iter_mut().find(|(m, _)| *m == method) {
            tree.insert(path, value).map_err(InsertError)?;
        } else {
            let mut tree = matchit::Router::new();
            tree.insert(path, value).map_err(InsertError)?;
            self.trees.push((method, tree));
        }
        Ok(())
    }

    /// Look up a route by HTTP method and path.
    ///
    /// Returns `None` if no route matches. The returned [`Match`] contains
    /// a reference to the stored value and zero-copy [`Params`].
    pub fn find<'k, 'v>(&'k self, method: &Method, path: &'v str) -> Option<Match<'k, 'v, V>>
    where
        'k: 'v,
    {
        self.trees
            .iter()
            .find(|(m, _)| m == method)
            .and_then(|(_, tree)| tree.at(path).ok())
            .map(move |m| Match {
                value: m.value,
                params: Params(m.params),
            })
    }

    // ── Builder methods (consume self for chaining) ──

    /// Register a `GET` route.
    pub fn get(mut self, path: &str, value: V) -> Self {
        self.insert(Method::GET, path, value)
            .expect("failed to insert GET route");
        self
    }

    /// Register a `POST` route.
    pub fn post(mut self, path: &str, value: V) -> Self {
        self.insert(Method::POST, path, value)
            .expect("failed to insert POST route");
        self
    }

    /// Register a `PUT` route.
    pub fn put(mut self, path: &str, value: V) -> Self {
        self.insert(Method::PUT, path, value)
            .expect("failed to insert PUT route");
        self
    }

    /// Register a `DELETE` route.
    pub fn delete(mut self, path: &str, value: V) -> Self {
        self.insert(Method::DELETE, path, value)
            .expect("failed to insert DELETE route");
        self
    }

    /// Register a `PATCH` route.
    pub fn patch(mut self, path: &str, value: V) -> Self {
        self.insert(Method::PATCH, path, value)
            .expect("failed to insert PATCH route");
        self
    }

    /// Register a `HEAD` route.
    pub fn head(mut self, path: &str, value: V) -> Self {
        self.insert(Method::HEAD, path, value)
            .expect("failed to insert HEAD route");
        self
    }

    /// Register an `OPTIONS` route.
    pub fn options(mut self, path: &str, value: V) -> Self {
        self.insert(Method::OPTIONS, path, value)
            .expect("failed to insert OPTIONS route");
        self
    }

    /// Register a route for an arbitrary HTTP method.
    pub fn route(mut self, method: Method, path: &str, value: V) -> Self {
        self.insert(method, path, value)
            .expect("failed to insert route");
        self
    }

    /// Group routes under a common path prefix.
    ///
    /// Scopes can be nested arbitrarily. All routes registered inside the
    /// scope closure will have the prefix prepended automatically.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use atomic_http::router::Router;
    ///
    /// enum Route { ListUsers, GetUser, CreateUser }
    ///
    /// let router = Router::new()
    ///     .scope("/api/v1", |s| s
    ///         .scope("/users", |s| s
    ///             .get("/", Route::ListUsers)
    ///             .get("/{id}", Route::GetUser)
    ///             .post("/", Route::CreateUser)
    ///         )
    ///     );
    /// ```
    pub fn scope(
        mut self,
        prefix: &str,
        f: impl FnOnce(ScopeBuilder<V>) -> ScopeBuilder<V>,
    ) -> Self {
        let scope = ScopeBuilder {
            prefix: prefix.to_string(),
            entries: Vec::new(),
        };
        let scope = f(scope);
        for (method, path, value) in scope.entries {
            self.insert(method, &path, value)
                .expect("failed to insert scoped route");
        }
        self
    }
}

/// Builder for registering routes under a common path prefix.
///
/// Created by [`Router::scope`]. Supports nested scopes via
/// [`ScopeBuilder::scope`].
pub struct ScopeBuilder<V> {
    prefix: String,
    entries: Vec<(Method, String, V)>,
}

impl<V> ScopeBuilder<V> {
    fn push(mut self, method: Method, path: &str, value: V) -> Self {
        self.entries
            .push((method, format!("{}{}", self.prefix, path), value));
        self
    }

    /// Register a `GET` route under this scope's prefix.
    pub fn get(self, path: &str, value: V) -> Self {
        self.push(Method::GET, path, value)
    }

    /// Register a `POST` route under this scope's prefix.
    pub fn post(self, path: &str, value: V) -> Self {
        self.push(Method::POST, path, value)
    }

    /// Register a `PUT` route under this scope's prefix.
    pub fn put(self, path: &str, value: V) -> Self {
        self.push(Method::PUT, path, value)
    }

    /// Register a `DELETE` route under this scope's prefix.
    pub fn delete(self, path: &str, value: V) -> Self {
        self.push(Method::DELETE, path, value)
    }

    /// Register a `PATCH` route under this scope's prefix.
    pub fn patch(self, path: &str, value: V) -> Self {
        self.push(Method::PATCH, path, value)
    }

    /// Register a `HEAD` route under this scope's prefix.
    pub fn head(self, path: &str, value: V) -> Self {
        self.push(Method::HEAD, path, value)
    }

    /// Register an `OPTIONS` route under this scope's prefix.
    pub fn options(self, path: &str, value: V) -> Self {
        self.push(Method::OPTIONS, path, value)
    }

    /// Register a route for an arbitrary HTTP method under this scope's prefix.
    pub fn route(self, method: Method, path: &str, value: V) -> Self {
        self.push(method, path, value)
    }

    /// Create a nested scope with an additional prefix.
    pub fn scope(
        mut self,
        prefix: &str,
        f: impl FnOnce(ScopeBuilder<V>) -> ScopeBuilder<V>,
    ) -> Self {
        let nested = ScopeBuilder {
            prefix: format!("{}{}", self.prefix, prefix),
            entries: Vec::new(),
        };
        let nested = f(nested);
        self.entries.extend(nested.entries);
        self
    }
}

impl<V> Default for Router<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Route {
        Home,
        GetUser,
        CreateUser,
        ServeFile,
    }

    #[test]
    fn basic_routing() {
        let router = Router::new()
            .get("/", Route::Home)
            .get("/users/{id}", Route::GetUser)
            .post("/users", Route::CreateUser);

        let m = router.find(&Method::GET, "/").unwrap();
        assert_eq!(*m.value, Route::Home);
        assert!(m.params.is_empty());

        let m = router.find(&Method::GET, "/users/42").unwrap();
        assert_eq!(*m.value, Route::GetUser);
        assert_eq!(m.params.get("id"), Some("42"));

        let m = router.find(&Method::POST, "/users").unwrap();
        assert_eq!(*m.value, Route::CreateUser);
    }

    #[test]
    fn wildcard_params() {
        let router = Router::new().get("/files/{*path}", Route::ServeFile);

        let m = router.find(&Method::GET, "/files/images/logo.png").unwrap();
        assert_eq!(*m.value, Route::ServeFile);
        assert_eq!(m.params.get("path"), Some("images/logo.png"));
    }

    #[test]
    fn method_mismatch() {
        let router = Router::new().get("/users", Route::GetUser);

        assert!(router.find(&Method::POST, "/users").is_none());
    }

    #[test]
    fn not_found() {
        let router = Router::new().get("/", Route::Home);

        assert!(router.find(&Method::GET, "/nonexistent").is_none());
    }

    #[test]
    fn multiple_params() {
        let router = Router::new().get("/orgs/{org}/repos/{repo}", Route::GetUser);

        let m = router
            .find(&Method::GET, "/orgs/acme/repos/widget")
            .unwrap();
        assert_eq!(m.params.get("org"), Some("acme"));
        assert_eq!(m.params.get("repo"), Some("widget"));
        assert_eq!(m.params.len(), 2);
    }

    #[test]
    fn same_path_different_methods() {
        let router = Router::new()
            .get("/users", Route::GetUser)
            .post("/users", Route::CreateUser);

        let m = router.find(&Method::GET, "/users").unwrap();
        assert_eq!(*m.value, Route::GetUser);

        let m = router.find(&Method::POST, "/users").unwrap();
        assert_eq!(*m.value, Route::CreateUser);
    }

    #[test]
    fn insert_returns_error_on_conflict() {
        let mut router = Router::<Route>::new();
        router
            .insert(Method::GET, "/users/{id}", Route::GetUser)
            .unwrap();
        assert!(router
            .insert(Method::GET, "/users/{name}", Route::GetUser)
            .is_err());
    }

    #[test]
    fn params_iter() {
        let router = Router::new().get("/a/{x}/b/{y}", Route::Home);

        let m = router.find(&Method::GET, "/a/1/b/2").unwrap();
        let pairs: Vec<_> = m.params.iter().collect();
        assert_eq!(pairs, vec![("x", "1"), ("y", "2")]);
    }

    #[test]
    fn scope_basic() {
        let router = Router::new().get("/", Route::Home).scope("/api", |s| {
            s.get("/users", Route::GetUser)
                .post("/users", Route::CreateUser)
        });

        let m = router.find(&Method::GET, "/api/users").unwrap();
        assert_eq!(*m.value, Route::GetUser);

        let m = router.find(&Method::POST, "/api/users").unwrap();
        assert_eq!(*m.value, Route::CreateUser);

        // Root still works
        let m = router.find(&Method::GET, "/").unwrap();
        assert_eq!(*m.value, Route::Home);
    }

    #[test]
    fn scope_nested() {
        let router = Router::new().scope("/api/v1", |s| {
            s.scope("/users", |s| {
                s.get("/", Route::GetUser)
                    .get("/{id}", Route::GetUser)
                    .post("/", Route::CreateUser)
            })
            .scope("/files", |s| s.get("/{*path}", Route::ServeFile))
        });

        let m = router.find(&Method::GET, "/api/v1/users/").unwrap();
        assert_eq!(*m.value, Route::GetUser);

        let m = router.find(&Method::GET, "/api/v1/users/42").unwrap();
        assert_eq!(m.params.get("id"), Some("42"));

        let m = router
            .find(&Method::GET, "/api/v1/files/img/logo.png")
            .unwrap();
        assert_eq!(m.params.get("path"), Some("img/logo.png"));
    }
}
