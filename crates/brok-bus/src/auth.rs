//! Capability-based authentication.
//!
//! Every client presents a random session token in its `Hello`. The bus maps
//! the token to a [`Grant`] that fixes the client's identity and the topics it
//! may publish and subscribe to. Shards get a fresh token per process,
//! derived from their manifest; the CLI uses the operator token written to
//! `.brokspace/run/operator.token`.
//!
//! Tokens are stored hashed (SHA-256), so lookups never compare secrets byte
//! by byte and a memory dump of the table does not reveal usable tokens.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use brok_sdk::frame::ShardContext;
use brok_sdk::topic::SYSTEM_NAMESPACE;
use brok_sdk::{ShardManifest, Topic, TopicPattern};
use sha2::{Digest, Sha256};

/// Kind of principal behind a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The local user through the CLI: may use any non-system topic.
    Operator,
    /// A service built into the runtime (e.g. the index).
    Service,
    /// A third-party or system shard, restricted by its manifest.
    Shard,
}

/// What a session is allowed to do.
#[derive(Debug, Clone)]
pub struct Grant {
    /// Authenticated name, stamped as `source` on every published event.
    pub identity: String,
    /// Principal kind.
    pub role: Role,
    /// Topics the session may publish to.
    pub publish: Vec<TopicPattern>,
    /// Topics the session may subscribe to.
    pub subscribe: Vec<TopicPattern>,
    /// Context returned in the `Welcome` frame.
    pub context: ShardContext,
}

impl Grant {
    /// Full access to every non-system topic.
    pub fn operator(context: ShardContext) -> Self {
        Self {
            identity: "operator".into(),
            role: Role::Operator,
            publish: vec![TopicPattern::any()],
            subscribe: vec![TopicPattern::any()],
            context,
        }
    }

    /// A built-in service with explicit topic lists.
    pub fn service(identity: &str, publish: Vec<TopicPattern>, subscribe: Vec<TopicPattern>) -> Self {
        Self { identity: identity.into(), role: Role::Service, publish, subscribe, context: ShardContext::default() }
    }

    /// The grant described by a shard manifest.
    pub fn for_shard(manifest: &ShardManifest, context: ShardContext) -> Self {
        Self {
            identity: manifest.name.clone(),
            role: Role::Shard,
            publish: manifest.publishes_to.clone(),
            subscribe: manifest.subscribes_to.clone(),
            context,
        }
    }

    /// Whether the session may publish on `topic`. The `system:` namespace
    /// is reserved for the bus itself.
    pub fn can_publish(&self, topic: &Topic) -> bool {
        topic.namespace() != SYSTEM_NAMESPACE && self.publish.iter().any(|p| p.matches(topic))
    }

    /// Whether the session may subscribe to `pattern`.
    pub fn can_subscribe(&self, pattern: &TopicPattern) -> bool {
        self.subscribe.iter().any(|grant| pattern.is_covered_by(grant))
    }
}

/// A secret session token. Its `Debug` output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    /// Generates 256 random bits, hex encoded.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the OS random number generator is unavailable");
        Self(hex::encode(bytes))
    }

    /// The secret value. Hand it only to the process it belongs to.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(<redacted>)")
    }
}

type TokenHash = [u8; 32];

fn hash(token: &str) -> TokenHash {
    Sha256::digest(token.as_bytes()).into()
}

/// Token → grant table.
#[derive(Debug, Default)]
pub struct Authenticator {
    grants: RwLock<HashMap<TokenHash, Arc<Grant>>>,
}

impl Authenticator {
    /// Creates an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Issues a new token for `grant`.
    pub fn issue(&self, grant: Grant) -> SessionToken {
        let token = SessionToken::generate();
        self.grants.write().unwrap_or_else(|e| e.into_inner()).insert(hash(token.expose()), Arc::new(grant));
        token
    }

    /// Revokes a token. Existing sessions are not affected.
    pub fn revoke(&self, token: &SessionToken) {
        self.grants.write().unwrap_or_else(|e| e.into_inner()).remove(&hash(token.expose()));
    }

    /// Resolves a presented token.
    pub fn authenticate(&self, token: &str) -> Option<Arc<Grant>> {
        self.grants.read().unwrap_or_else(|e| e.into_inner()).get(&hash(token)).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> TopicPattern {
        TopicPattern::new(s).unwrap()
    }

    #[test]
    fn tokens_are_unique_and_redacted() {
        let a = SessionToken::generate();
        assert_ne!(a, SessionToken::generate());
        assert_eq!(a.expose().len(), 64);
        assert!(!format!("{a:?}").contains(a.expose()));
    }

    #[test]
    fn authenticate_and_revoke() {
        let auth = Authenticator::new();
        let token = auth.issue(Grant::operator(ShardContext::default()));
        assert_eq!(auth.authenticate(token.expose()).unwrap().identity, "operator");
        assert!(auth.authenticate("guess").is_none());
        auth.revoke(&token);
        assert!(auth.authenticate(token.expose()).is_none());
    }

    #[test]
    fn system_namespace_is_reserved() {
        let operator = Grant::operator(ShardContext::default());
        assert!(operator.can_publish(&Topic::new("file:changed").unwrap()));
        assert!(!operator.can_publish(&Topic::new("system:shutdown").unwrap()));
    }

    #[test]
    fn shard_grants_follow_the_manifest() {
        let grant = Grant::service("svc", vec![p("index:*")], vec![p("file:changed")]);
        assert!(grant.can_publish(&Topic::new("index:updated").unwrap()));
        assert!(!grant.can_publish(&Topic::new("file:changed").unwrap()));
        assert!(grant.can_subscribe(&p("file:changed")));
        assert!(!grant.can_subscribe(&p("file:*")));
    }
}
