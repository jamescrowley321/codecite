; Definitions that become chunks: @function, @type, @variable or @alias,
; each with its @name.
; An `impl` block is a @scope: its methods are named after its type.
(function_item name: (identifier) @name) @function
(function_signature_item name: (identifier) @name) @function
(struct_item name: (type_identifier) @name) @type
(enum_item name: (type_identifier) @name) @type
(union_item name: (type_identifier) @name) @type
(trait_item name: (type_identifier) @name) @type
(impl_item type: (_) @name) @scope
(const_item name: (identifier) @name) @variable
(static_item name: (identifier) @name) @variable
(type_item name: (type_identifier) @name) @alias
