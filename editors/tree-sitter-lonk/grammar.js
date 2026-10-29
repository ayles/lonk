const commaSep = rule => optional(seq(rule, repeat(seq(',', rule)), optional(',')));
// Record fields, type fields and match arms are separated by `;`, `,` or a line break,
// exactly like syntax.rs::separator. The line break is an external token so that an
// expression continuing on the next line (`f\n(1)`, `a\n+ b`) is still one entry.
const separated = ($, rule) => optional(seq(rule, repeat(seq(choice(';', ',', $._newline), rule)), optional(choice(';', ','))));

module.exports = grammar({
  name: 'lonk',
  extras: $ => [/\s/, $.line_comment, $.block_comment],
  // `_error_sentinel` is never used in a rule: it is only valid during error recovery,
  // which tells the scanner not to emit zero-width separators there.
  externals: $ => [$.block_comment, $._newline, $._error_sentinel],
  word: $ => $.identifier,
  conflicts: $ => [[$._statement_block, $.block], [$._statement_block, $.record, $.block], [$._type, $.generic_type], [$.record, $.block], [$._expression, $.record_field], [$.call_expression, $.unary_expression, $.binary_expression], [$._expression, $._type_identifier], [$._expression, $.literal_type], [$.call_expression, $.binary_expression], [$.record, $.block, $.record_type], [$.parameter, $._type_identifier], [$.parameters, $.function_type]],
  rules: {
    source_file: $ => seq(repeat($._statement), optional($._expression)),
    _statement: $ => choice($.let_statement, $.assignment_statement, $.return_statement,
      $.expression_statement, $.for_statement, $.block_statement),
    let_statement: $ => seq('let', field('name', $.identifier), optional(seq(':', field('type', $._type))), '=', field('value', $._expression), ';'),
    assignment_statement: $ => seq(field('target', $._expression), field('operator', choice('=', '+=', '-=', '*=', '/=')), field('value', $._expression), ';'),
    return_statement: $ => seq('return', optional(field('value', $._expression)), ';'),
    expression_statement: $ => seq($._expression, ';'),
    block_statement: $ => seq(alias($._statement_block, $.block), optional(';')),
    _statement_block: $ => seq('{', repeat($._statement), '}'),
    for_statement: $ => prec(1, seq($.for_expression, optional(';'))),
    _expression: $ => choice($.identifier, $.integer, $.boolean, $.string, $.record, $.array,
      $.function_expression, $.type_expression, $.call_expression, $.field_expression,
      $.index_expression, $.slice_expression, $.unary_expression, $.binary_expression,
      $.parenthesized_expression, $.block, $.if_expression, $.match_expression, $.for_expression),
    identifier: _ => /[A-Za-z_][A-Za-z_0-9]*/,
    integer: _ => /[0-9]+/,
    boolean: _ => choice('true', 'false'),
    // `%{` is reserved for future interpolation and rejected by the checker; the grammar
    // shows it as ordinary string content (see README).
    string: $ => seq('"', repeat(choice($.escape_sequence, $.string_content)), '"'),
    escape_sequence: _ => token.immediate(seq('\\', /[nrt\\"%]/)),
    string_content: _ => token.immediate(/[^"\\]+/),
    line_comment: _ => token(seq('//', /[^\n]*/)),
    parenthesized_expression: $ => seq('(', $._expression, ')'),
    record: $ => prec.dynamic(1, seq('{', separated($, choice($.record_field, $.spread)), '}')),
    record_field: $ => seq(field('name', choice($.identifier, $.string)), '=', field('value', $._expression)),
    spread: $ => seq('...', $._expression),
    array: $ => seq('[', commaSep(choice($._expression, $.spread)), ']'),
    block: $ => seq('{', repeat($._statement), optional($._expression), '}'),
    function_expression: $ => prec.right(seq('fn', optional($.type_parameters), $.parameters,
      choice(
        seq('->', field('result', $._type), field('body', $.block)),
        field('body', choice(prec(1, $.block), $._expression))))),
    type_parameters: $ => seq('<', commaSep($.identifier), '>'),
    parameters: $ => seq('(', commaSep($.parameter), ')'),
    // Only `&self` has the borrow shortcut; every other borrowed parameter is `name: &Type`.
    parameter: $ => choice(
      seq('&', field('name', alias('self', $.identifier))),
      seq(field('name', $.identifier), optional(seq(':', field('type', $._type))), optional(seq('=', field('default', $._expression))))),
    type_expression: $ => seq('type', optional($.type_parameters), $._type),
    call_expression: $ => prec.left(9, seq(field('function', $._expression), optional($.call_type_arguments), $.arguments)),
    // `f<int>(1)` and `a < b` share a token. Both readings are kept and the generic call wins
    // whenever it parses, which is what syntax.rs does when `<` is adjacent to the callee.
    call_type_arguments: $ => prec.dynamic(1, seq('<', commaSep($._type), '>')),
    arguments: $ => seq('(', commaSep($._expression), ')'),
    field_expression: $ => prec.left(9, seq(field('value', $._expression), '.', field('field', $.identifier))),
    index_expression: $ => prec.left(9, seq(field('value', $._expression), '[', field('index', $._expression), ']')),
    slice_expression: $ => prec.left(9, seq(field('value', $._expression), '[', optional(field('start', $._expression)), ':', optional(field('end', $._expression)), ']')),
    unary_expression: $ => prec(8, seq(field('operator', choice('!', '-', '&')), field('operand', $._expression))),
    binary_expression: $ => choice(...[
      [1, '||'], [2, '&&'], [3, choice('==', '!=', 'in')],
      [4, choice('<', '<=', '>', '>=')], [5, choice('+', '-')], [6, choice('*', '/', '%')],
    ].map(([p, op]) => prec.left(p, seq(field('left', $._expression), field('operator', op), field('right', $._expression))))),
    if_expression: $ => prec.right(seq('if', field('condition', $._expression), field('consequence', $.block), optional(seq('else', field('alternative', choice($.block, $.if_expression)))))),
    for_expression: $ => seq('for', field('binding', $.identifier), 'in', field('source', $._expression), field('body', $.block)),
    match_expression: $ => seq('match', field('value', $._expression), '{', separated($, $.match_arm), '}'),
    match_arm: $ => seq(field('pattern', choice('else', $._type)), '=>', field('body', $._expression)),
    _type: $ => choice($._type_identifier, $.type_path, $.literal_type, $.record_type, $.array_type,
      $.borrow_type, $.function_type, $.generic_type, $.union_type, $.parenthesized_type),
    _type_identifier: $ => alias($.identifier, $.type_identifier),
    // `module.Name`: a type value reached through object fields.
    type_path: $ => prec.left(1, seq($._type_identifier, repeat1(seq('.', $._type_identifier)))),
    literal_type: $ => choice($.string, $.boolean, $.integer, seq('-', $.integer)),
    record_type: $ => seq('{', optional(seq($.type_field, repeat(seq(choice(';', ',', $._newline), $.type_field)), optional(choice(';', ',', $._newline)))), optional($.row_type), '}'),
    type_field: $ => seq(field('name', choice($.identifier, $.string)), optional('?'), ':', field('type', $._type)),
    row_type: $ => seq('...', optional($._type_identifier), optional(choice(';', ','))),
    array_type: $ => seq('[', $._type, ']'),
    borrow_type: $ => prec.right(seq('&', $._type)),
    function_type: $ => prec.right(seq('fn', optional($.type_parameters), '(', commaSep($.function_type_parameter), ')', optional(seq('->', $._type)))),
    function_type_parameter: $ => seq(optional(seq('self', ':')), $._type, optional(seq('=', '...'))),
    generic_type: $ => seq(choice($._type_identifier, $.type_path), $.type_arguments),
    type_arguments: $ => seq('<', commaSep($._type), '>'),
    union_type: $ => prec.left(seq($._type, '|', $._type)),
    parenthesized_type: $ => seq('(', $._type, ')'),
  }
});
