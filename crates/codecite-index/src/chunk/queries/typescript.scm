; Definitions that become chunks: @function or @type, each with its @name.
(function_declaration name: (identifier) @name) @function
(generator_function_declaration name: (identifier) @name) @function
(function_signature name: (identifier) @name) @function
(method_definition name: (_) @name) @function
(abstract_method_signature name: (_) @name) @function
(variable_declarator
  name: (identifier) @name
  value: [(arrow_function) (function_expression)]) @function
(class_declaration name: (_) @name) @type
(abstract_class_declaration name: (_) @name) @type
(interface_declaration name: (_) @name) @type
(enum_declaration name: (_) @name) @type
