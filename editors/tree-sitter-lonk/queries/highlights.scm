(identifier) @variable
["let" "fn" "type" "return" "if" "else" "match" "for" "in"] @keyword
(integer) @number
(boolean) @constant.builtin
(string) @string
(escape_sequence) @string.escape
(line_comment) @comment
(block_comment) @comment
(type_identifier) @type
(type_parameters (identifier) @type.parameter)
(parameter name: (identifier) @variable.parameter)
(record_field name: (identifier) @property)
(type_field name: (identifier) @property)
(field_expression field: (identifier) @property)
(call_expression function: (identifier) @function)
(call_expression function: (field_expression field: (identifier) @function.method))
(let_statement name: (identifier) @function value: (function_expression))
(record_field name: (identifier) @function.method value: (function_expression))
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
[";" "," "." ":"] @punctuation.delimiter
["=" "+=" "-=" "*=" "/=" "->" "=>" "..." "?" "&" "!" "-" "+" "*" "/" "%" "==" "!=" "<" "<=" ">" ">=" "&&" "||" "|"] @operator
((identifier) @variable.builtin (#eq? @variable.builtin "self"))
((type_identifier) @type.builtin (#any-of? @type.builtin "int" "str" "bool" "any"))
