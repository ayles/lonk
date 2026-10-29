(source_file) @local.scope
(block) @local.scope
(function_expression) @local.scope
(let_statement name: (identifier) @local.definition)
(parameter name: (identifier) @local.definition)
(for_expression binding: (identifier) @local.definition)
(identifier) @local.reference
