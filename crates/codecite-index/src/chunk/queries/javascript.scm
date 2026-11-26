; Definitions that become chunks: @function, @type, @variable or @alias,
; each with its @name. A @variable whose @value is a function is that
; function's definition instead.
(function_declaration name: (identifier) @name) @function
(generator_function_declaration name: (identifier) @name) @function
; A method in a class, or in an object literal that a variable, an
; assignment or `export default` names. One in an argument, an array
; or a nested object stays in the chunk around it.
(class_body (method_definition name: (_) @name) @function)
(variable_declarator value: (object (method_definition name: (_) @name) @function))
(assignment_expression right: (object (method_definition name: (_) @name) @function))
(export_statement value: (object (method_definition name: (_) @name) @function))
(variable_declarator
  name: (identifier) @name
  value: [(arrow_function) (function_expression)]) @function
; A class property holding a function is a method.
(field_definition
  property: (_) @name
  value: [(arrow_function) (function_expression)]) @function
; `X.prototype.m = function` is a method of `X`.
(expression_statement
  (assignment_expression
    left: (member_expression
      object: (member_expression
        object: (identifier) @receiver
        property: (property_identifier) @_proto)
      property: (property_identifier) @name)
    right: [(arrow_function) (function_expression)])
  (#eq? @_proto "prototype")) @function
(class_declaration name: (_) @name) @type
(class name: (_) @name) @type
; Top-level `const`, `let` and `var`, exported or not.
(program (lexical_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable))
(program (variable_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable))
(program (export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable)))
(program (export_statement declaration: (variable_declaration (variable_declarator name: (identifier) @name value: (_)? @value) @variable)))
