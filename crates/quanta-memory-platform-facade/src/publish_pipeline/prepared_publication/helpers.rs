use super::*;

pub(super) fn normalize_referenced_tokens(
    referenced_tokens: &[PreparedPublicationTokenV1],
) -> Result<BTreeSet<PreparedPublicationTokenV1>, PreparedPublicationErrorV1> {
    if referenced_tokens.len() > MAX_REFERENCED_ROOT_TOKENS {
        return Err(PreparedPublicationErrorV1::Storage(MemoryError::capacity(format!(
            "prepared publication referenced-root slice exceeds the maximum of \
             {MAX_REFERENCED_ROOT_TOKENS} raw tokens (got {})",
            referenced_tokens.len()
        ))));
    }
    Ok(referenced_tokens.iter().cloned().collect())
}

/// Validate every referenced token as live and non-aborted, then return the
/// prepared entries absent from the normalized reference set. An unknown or
/// aborted reference rejects the whole classification before any entry is
/// chosen or removed, so GC can never partially collect a valid store.
pub(super) fn classify_orphan_tokens<Payload>(
    state: &PreparedState<Payload>,
    referenced_tokens: &BTreeSet<PreparedPublicationTokenV1>,
) -> Result<Vec<PreparedPublicationTokenV1>, PreparedPublicationErrorV1> {
    for token in referenced_tokens {
        if state.aborted_tokens.contains(token) {
            return Err(PreparedPublicationErrorV1::AbortedToken { token: token.clone() });
        }
        if !state.entries.contains_key(token) {
            return Err(PreparedPublicationErrorV1::UnknownToken { token: token.clone() });
        }
    }
    Ok(state.entries.keys().filter(|token| !referenced_tokens.contains(*token)).cloned().collect())
}

pub(super) fn derive_token(
    identity: &PreparedPublicationIdentityV1,
    payload_key: &[u8],
) -> Result<PreparedPublicationTokenV1, PreparedPublicationErrorV1> {
    let identity_len = measured_len(identity.0.len())?;
    let key_len = measured_len(payload_key.len())?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(TOKEN_DOMAIN);
    hasher.update(&identity_len.to_le_bytes());
    hasher.update(&identity.0);
    hasher.update(&key_len.to_le_bytes());
    hasher.update(payload_key);
    Ok(PreparedPublicationTokenV1(*hasher.finalize().as_bytes()))
}
