; Definitions that become chunks: @function, @type, @variable or @alias,
; each with its @name.
; A method's @receiver names the type it belongs to.
(function_declaration name: (identifier) @name) @function
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [(type_identifier) @receiver
             (pointer_type (type_identifier) @receiver)
             (generic_type type: (type_identifier) @receiver)
             (pointer_type (generic_type type: (type_identifier) @receiver))]))
  name: (field_identifier) @name) @function
(method_elem name: (field_identifier) @name) @function
(type_spec name: (type_identifier) @name type: [(struct_type) (interface_type)]) @type
; Any other named type (`type Mode int`, `type Handler func()`) and `type A = B`.
(type_spec name: (type_identifier) @name type: (_) @value) @alias
(type_alias name: (type_identifier) @name) @alias
; Package-level `const` and `var`, one per name.
(source_file (const_declaration (const_spec name: (identifier) @name) @variable))
(source_file (var_declaration (var_spec name: (identifier) @name) @variable))
(source_file (var_declaration (var_spec_list (var_spec name: (identifier) @name) @variable)))
