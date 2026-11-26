; Function definitions only: declarations ending in a semicolon are prototypes.
(function_definition) @c_function
(struct_specifier name: (type_identifier) @name body: (field_declaration_list)) @type
(union_specifier name: (type_identifier) @name body: (field_declaration_list)) @type
(enum_specifier name: (type_identifier) @name body: (enumerator_list)) @type
(type_definition declarator: (type_identifier) @name) @alias
; A function-like macro. Object-like ones (`#define REG 0x1c8`) are too
; many to be definitions.
(preproc_function_def name: (identifier) @name) @function
; A file-scope table: a declaration with a brace initialiser, at the top of
; the file or inside a preprocessor conditional there.
(translation_unit (declaration declarator: (init_declarator value: (initializer_list)) @c_declarator) @c_variable)
(preproc_if (declaration declarator: (init_declarator value: (initializer_list)) @c_declarator) @c_variable)
(preproc_ifdef (declaration declarator: (init_declarator value: (initializer_list)) @c_declarator) @c_variable)
(preproc_else (declaration declarator: (init_declarator value: (initializer_list)) @c_declarator) @c_variable)
(preproc_elif (declaration declarator: (init_declarator value: (initializer_list)) @c_declarator) @c_variable)
