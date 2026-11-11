use crate::indexer::Indexer;
use crate::models::index_options::IndexOptions;
use crate::models::project_ref::ProjectRef;
use crate::store::MemoryStore;
use std::path::Path;

pub const PROJECT: &str = "fixture";

pub const GRAPH_JSON: &str = r#"{
 "nodes": [
  {"id":"pm","label":"PlayerMovement","source_file":"Assets/Scripts/PlayerMovement.cs","source_location":"L5"},
  {"id":"jump","label":"Jump","source_file":"Assets/Scripts/PlayerMovement.cs","source_location":"L14"},
  {"id":"dmg","label":"ApplyDamage","source_file":"Assets/Scripts/Health.cs","source_location":"L8"}
 ],
 "links": [
  {"source":"jump","target":"dmg","relation":"calls"}
 ]
}"#;

pub const PLAYER_CS: &str = "using UnityEngine;

public class PlayerMovement : MonoBehaviour
{
    public float jumpHeight = 3f;

    private void Update()
    {
        if (Input.GetButtonDown(\"Jump\")) Jump();
    }

    public void Jump()
    {
        Health.ApplyDamage(1);
    }
}
";

pub const HEALTH_CS: &str = "public static class Health
{
    public static int points = 3;

    public static void ApplyDamage(int amount)
    {
        points -= amount;
    }
}
";

pub fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub fn fixture_project(root: &Path) -> ProjectRef {
    write(
        root,
        ".claude/BIBLE/03_heroine/capacites.md",
        "# Capacités de l'héroïne\n\n## Double saut\nL'héroïne peut effectuer un double saut après avoir quitté le sol.\n\n## Dash\nLe dash est une esquive rapide.\n",
    );
    write(
        root,
        "Docs/GameDesign/boss.md",
        "# Boss de la forêt\nLe gardien de la forêt attaque en trois phases.\n",
    );
    write(root, "Assets/Scripts/PlayerMovement.cs", PLAYER_CS);
    write(root, "Assets/Scripts/Health.cs", HEALTH_CS);
    write(
        root,
        "Library/Cache/junk.cs",
        "class Junk { void Nope() {} }",
    );
    write(root, "graphify-out/graph.json", GRAPH_JSON);
    write(root, "README.md", "# Fixture\nUn mini projet.\n");
    ProjectRef {
        key: PROJECT.to_string(),
        root: root.to_path_buf(),
    }
}

pub fn indexed_store(root: &Path) -> (MemoryStore, ProjectRef) {
    let project = fixture_project(root);
    let store = MemoryStore::open_in_memory().unwrap();
    Indexer::new(&store)
        .index(&project, &IndexOptions::default())
        .unwrap();
    (store, project)
}

pub const FAKE_MODEL: &str = "fake-model";
pub const FAKE_DIM: usize = 64;
const SYNONYMS: &[(&str, &str)] = &[("bondir", "saut"), ("sauter", "saut"), ("jump", "saut")];
const HASH_SEED: usize = 17;
const HASH_FACTOR: usize = 31;

/// Deterministic bag-of-words embedder: words hash into buckets, synonyms share a bucket.
pub struct HashEmbedder {
    pub id: String,
}

impl HashEmbedder {
    pub fn new(id: &str) -> Self {
        Self { id: id.to_string() }
    }

    fn vector(text: &str) -> Vec<f32> {
        let mut vector = vec![0.0_f32; FAKE_DIM];
        for word in text
            .to_lowercase()
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
        {
            let canonical = SYNONYMS
                .iter()
                .find(|(alias, _)| *alias == word)
                .map_or(word, |(_, target)| *target);
            let bucket = canonical.bytes().fold(HASH_SEED, |acc, byte| {
                acc.wrapping_mul(HASH_FACTOR)
                    .wrapping_add(usize::from(byte))
            }) % FAKE_DIM;
            vector[bucket] += 1.0;
        }
        vector
    }
}

impl crate::embedder::Embedder for HashEmbedder {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn dim(&self) -> usize {
        FAKE_DIM
    }

    fn embed(&self, texts: &[&str]) -> crate::error::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|text| Self::vector(text)).collect())
    }
}
