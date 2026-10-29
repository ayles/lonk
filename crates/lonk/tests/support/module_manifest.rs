use lonk::ModuleSource;
use std::{collections::BTreeMap, path::Path};

/// This explicit manifest is host policy, not language import syntax.
pub fn load(directory: &Path) -> Result<BTreeMap<String, ModuleSource>, std::io::Error> {
    let manifest: &[(&str, &[(&str, &str)])] = &[
        ("types", &[]),
        ("fixpoint", &[]),
        ("compiler", &[("types", "types")]),
        ("zlib", &[("types", "types")]),
        ("editor", &[("types", "types")]),
        (
            "catalog",
            &[
                ("compiler", "compiler"),
                ("zlib", "zlib"),
                ("editor", "editor"),
            ],
        ),
        ("packages", &[("catalog", "catalog")]),
        ("services", &[("types", "types")]),
        ("merge", &[("types", "types")]),
        (
            "system",
            &[
                ("types", "types"),
                ("catalog", "catalog"),
                ("services", "services"),
                ("merge", "merge"),
            ],
        ),
        (
            "main",
            &[
                ("packages", "packages"),
                ("system", "system"),
                ("fixpoint", "fixpoint"),
            ],
        ),
    ];
    let mut modules = BTreeMap::new();
    for (name, dependencies) in manifest {
        let source = std::fs::read_to_string(directory.join(format!("{name}.lonk")))?;
        let mut module = ModuleSource::new(source);
        for (binding, target) in *dependencies {
            module = module.with_dependency(*binding, *target);
        }
        modules.insert((*name).to_owned(), module);
    }
    Ok(modules)
}
