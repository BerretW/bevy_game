//! Dependency resolver.
//!
//! Z manifest mapy poskládá load-order: každý resource musí být nahrán
//! až *poté*, co všechny jeho dependencies. Standard Kahn's algorithm
//! (topological sort přes in-degree). Detekuje:
//!
//! * chybějící dependency (manifest se odkazuje na neexistující ID),
//! * cyklus (po Kahn's zůstanou nedosažené uzly),
//! * sám-na-sebe odkaz (specifická forma cyklu, hlásíme zvlášť).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::manifest::Manifest;
use crate::types::ResourceId;

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("resource {dependent} depends on missing resource {missing}")]
    MissingDependency {
        dependent: ResourceId,
        missing: ResourceId,
    },
    #[error("resource {0} declares itself as a dependency")]
    SelfDependency(ResourceId),
    #[error("dependency cycle detected involving: {0:?}")]
    Cycle(Vec<ResourceId>),
}

/// Vrátí ID resources v pořadí, ve kterém je bezpečné spouštět skripty.
pub fn resolve_load_order(
    manifests: &HashMap<ResourceId, Manifest>,
) -> Result<Vec<ResourceId>, ResolveError> {
    // Validate references + collect adjacency.
    let mut in_degree: HashMap<ResourceId, usize> = HashMap::with_capacity(manifests.len());
    let mut reverse_edges: HashMap<ResourceId, Vec<ResourceId>> =
        HashMap::with_capacity(manifests.len());

    for id in manifests.keys() {
        in_degree.insert(id.clone(), 0);
        reverse_edges.entry(id.clone()).or_default();
    }

    for (id, manifest) in manifests {
        for dep in &manifest.dependencies {
            let dep_id = ResourceId::new(dep.clone());
            if &dep_id == id {
                return Err(ResolveError::SelfDependency(id.clone()));
            }
            if !manifests.contains_key(&dep_id) {
                return Err(ResolveError::MissingDependency {
                    dependent: id.clone(),
                    missing: dep_id,
                });
            }
            // edge: dep_id ──▶ id (dep_id musí být první)
            *in_degree.entry(id.clone()).or_insert(0) += 1;
            reverse_edges
                .entry(dep_id)
                .or_default()
                .push(id.clone());
        }
    }

    // Kahn's algorithm — startují uzly s in-degree 0,
    // řadíme je sortovaně, abychom dostali deterministický load-order.
    let mut zero: Vec<ResourceId> = in_degree
        .iter()
        .filter_map(|(id, &deg)| (deg == 0).then(|| id.clone()))
        .collect();
    zero.sort();
    let mut queue: VecDeque<ResourceId> = zero.into();

    let mut order = Vec::with_capacity(manifests.len());
    while let Some(id) = queue.pop_front() {
        order.push(id.clone());
        if let Some(successors) = reverse_edges.get(&id) {
            // Rovněž sortujeme pro determinismus
            let mut successors = successors.clone();
            successors.sort();
            for succ in successors {
                let entry = in_degree.get_mut(&succ).expect("invariant: in_degree contains every id");
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    queue.push_back(succ);
                }
            }
        }
    }

    if order.len() != manifests.len() {
        // Najdi uzly, které zůstaly v cyklu — to jsou ty s in-degree > 0.
        let cycle_nodes: HashSet<ResourceId> = in_degree
            .iter()
            .filter_map(|(id, &deg)| (deg > 0).then(|| id.clone()))
            .collect();
        let mut cycle_vec: Vec<ResourceId> = cycle_nodes.into_iter().collect();
        cycle_vec.sort();
        return Err(ResolveError::Cycle(cycle_vec));
    }

    Ok(order)
}
