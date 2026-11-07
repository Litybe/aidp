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
