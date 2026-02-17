use super::domain::MemoryNode;
use super::repository::MemoryRepository;

const EMBEDDING_DIM: usize = 64;

pub fn embed_query_text(text: &str) -> Option<Vec<f32>> {
    embed_text(text)
}

pub fn upsert_embeddings_for_nodes_best_effort(
    repo: &MemoryRepository,
    nodes: &[MemoryNode],
    updated_at: i64,
) {
    for node in nodes {
        let Some(embedding) = embed_text(&node.content) else {
            continue;
        };
        let _ = repo.upsert_embedding(&node.memory_id, &embedding, updated_at);
    }
}

fn embed_text(text: &str) -> Option<Vec<f32>> {
    let tokens = text
        .split_whitespace()
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return None;
    }

    let mut out = vec![0.0_f32; EMBEDDING_DIM];
    for token in tokens {
        let hash = stable_hash(token.as_bytes());
        let idx = (hash as usize) % EMBEDDING_DIM;
        let sign = if hash & 1 == 0 { 1.0_f32 } else { -1.0_f32 };
        let mag = 1.0_f32 + (token.len() as f32 / 32.0_f32);
        out[idx] += sign * mag;
    }

    let norm = out.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return None;
    }
    for value in &mut out {
        *value /= norm;
    }
    Some(out)
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3_u64);
    }
    hash
}
