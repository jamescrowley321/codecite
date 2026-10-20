; Definitions that become chunks: @function or @type, each with its @name.
(function_declaration name: (identifier) @name) @function
(generator_function_declaration name: (identifier) @name) @function
(method_definition name: (_) @name) @function
(variable_declarator
  name: (identifier) @name
  value: [(arrow_function) (function_expression)]) @function
(class_declaration name: (_) @name) @type
(class name: (_) @name) @type
