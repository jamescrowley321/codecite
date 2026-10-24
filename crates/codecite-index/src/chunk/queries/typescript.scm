; Definitions that become chunks: @function, @type, @variable or @alias,
; each with its @name. A @variable whose @value is a function is that
; function's definition instead.
(function_declaration name: (identifier) @name) @function
(generator_function_declaration name: (identifier) @name) @function
(function_signature name: (identifier) @name) @function
(method_definition name: (_) @name) @function
(abstract_method_signature name: (_) @name) @function
(method_signature name: (_) @name) @function
(variable_declarator
  name: (identifier) @name
  value: [(arrow_function) (function_expression)]) @function
(class_declaration name: (_) @name) @type
(class name: (_) @name) @type
(abstract_class_declaration name: (_) @name) @type
(interface_declaration name: (_) @name) @type
(enum_declaration name: (_) @name) @type
(type_alias_declaration name: (type_identifier) @name) @alias
; Top-level `const`, `let` and `var`, exported or not.
(program (lexical_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable))
(program (variable_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable))
(program (export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable)))
(program (export_statement declaration: (variable_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable)))
