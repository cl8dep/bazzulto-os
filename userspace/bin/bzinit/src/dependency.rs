//! Dependency-ordered service boot via Kahn's topological sort.

use alloc::vec;
use alloc::vec::Vec;
use alloc::string::String;
use crate::service::ServiceState;

/// Return indices into `services` in dependency-resolved boot order.
///
/// Services with no `after` dependencies come first. If a dependency cycle
/// is detected the remaining services are appended in input order so the
/// system still boots as far as possible.
pub fn topological_order(services: &[ServiceState]) -> Vec<usize> {
    let count = services.len();
    let mut in_degree = vec![0usize; count];
    // adjacency: dependents[i] = list of service indices that depend on i
    let mut dependents: Vec<Vec<usize>> = (0..count).map(|_| Vec::new()).collect();

    // Build in-degree and adjacency from `after` lists.
    for (dependent_index, service_state) in services.iter().enumerate() {
        for dependency_name in &service_state.definition.after {
            // Find the index of the named dependency.
            if let Some(dependency_index) = services
                .iter()
                .position(|other| &other.definition.name == dependency_name)
            {
                in_degree[dependent_index] += 1;
                dependents[dependency_index].push(dependent_index);
            }
            // Unknown dependencies are ignored — service still boots,
            // just without the ordering guarantee.
        }
    }

    // Kahn's algorithm — queue starts with all in-degree-0 services.
    let mut queue: Vec<usize> = (0..count)
        .filter(|&index| in_degree[index] == 0)
        .collect();
    let mut order: Vec<usize> = Vec::with_capacity(count);

    while let Some(current_index) = queue.first().copied() {
        queue.remove(0);
        order.push(current_index);

        for &dependent_index in &dependents[current_index] {
            in_degree[dependent_index] -= 1;
            if in_degree[dependent_index] == 0 {
                queue.push(dependent_index);
            }
        }
    }

    // Cycle detection: append any remaining services in input order.
    if order.len() < count {
        for index in 0..count {
            if !order.contains(&index) {
                order.push(index);
            }
        }
    }

    order
}
