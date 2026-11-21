; Definitions that become chunks: @function or @type, each with its @name.
(method_declaration name: (identifier) @name) @function
(constructor_declaration name: (identifier) @name) @function
(class_declaration name: (identifier) @name) @type
(interface_declaration name: (identifier) @name) @type
(enum_declaration name: (identifier) @name) @type
(record_declaration name: (identifier) @name) @type
(annotation_type_declaration name: (identifier) @name) @type
