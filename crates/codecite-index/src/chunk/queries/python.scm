; Definitions that become chunks: @function, @type, @variable or @alias,
; each with its @name.
(function_definition name: (identifier) @name) @function
(class_definition name: (identifier) @name) @type
; A decorated definition's chunk starts at its first decorator.
(decorated_definition) @decorated
; Module-level names: `NAME = ...`, `NAME: T = ...` and `type NAME = ...`.
(module (expression_statement (assignment left: (identifier) @name)) @variable)
(module (type_alias_statement left: (type (identifier) @name)) @alias)
