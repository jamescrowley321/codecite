; Definitions that become chunks: @function or @type, each with its @name.
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
(type_spec name: (type_identifier) @name type: [(struct_type) (interface_type)]) @type
