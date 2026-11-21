; Function definitions only: declarations ending in a semicolon are prototypes.
(function_definition) @c_function
(struct_specifier name: (type_identifier) @name body: (field_declaration_list)) @type
(union_specifier name: (type_identifier) @name body: (field_declaration_list)) @type
(enum_specifier name: (type_identifier) @name body: (enumerator_list)) @type
(type_definition declarator: (type_identifier) @name) @alias
