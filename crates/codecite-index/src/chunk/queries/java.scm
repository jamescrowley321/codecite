; Definitions that become chunks: @function or @type, each with its @name.
(method_declaration name: (identifier) @name) @function
(constructor_declaration name: (identifier) @name) @function
(class_declaration name: (identifier) @name) @type
(interface_declaration name: (identifier) @name) @type
