; Definitions that become chunks: @function or @type, each with its @name.
(function_definition name: (identifier) @name) @function
(class_definition name: (identifier) @name) @type
; A decorated definition's chunk starts at its first decorator.
(decorated_definition) @decorated
