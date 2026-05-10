// Toto je část, která je potřeba do hook.rs
    let gltf_mat_name = mat_names.get(entity).map(|m| m.0.as_str()).unwrap_or("");
    let Some(mat_def) = manifest.materials.get(gltf_mat_name) else {
        debug!(
            "[drawable] '{}': GLTF mat '{}' není v manifestu, swap přeskočen",
            node_name, gltf_mat_name
        );
        warn!(
            "[drawable] '{}': GLTF mat '{}' NOT FOUND in manifest. Available materials: {:?}",
            node_name, gltf_mat_name,
            manifest.materials.keys().collect::<Vec<_>>()
        );
        return;
    };
