@spec-6.4 @phase-1
Feature: n8n extension methods and Luxon dates
  Expressions ship n8n's extension methods on strings, numbers, arrays,
  objects and dates, plus Luxon (`DateTime`, `$now`, `$today`), and give the
  same results as n8n.

  Scenario Outline: String extensions
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                                                  | result                    |
      | ={{ 'hello world'.toSnakeCase() }}                          | "hello_world"             |
      | ={{ 'hello world'.toTitleCase() }}                          | "Hello World"             |
      | ={{ 'hello world'.toSentenceCase() }}                       | "Hello world"             |
      | ={{ 'Contact: ada@example.com today'.extractEmail() }}      | "ada@example.com"         |
      | ={{ 'https://n8n.io/pricing?x=1'.extractDomain() }}         | "n8n.io"                  |
      | ={{ 'see https://example.com/a now'.extractUrl() }}         | "https://example.com/a"   |
      | ={{ '<p>Hi <b>there</b></p>'.removeTags() }}                | "Hi there"                |
      | ={{ ''.isEmpty() }}                                         | true                      |
      | ={{ 'x'.isNotEmpty() }}                                     | true                      |
      | ={{ 'ada@example.com'.isEmail() }}                          | true                      |
      | ={{ '42'.toNumber() }}                                      | 42                        |
      | ={{ 'hello'.hash('md5') }}                                  | "5d41402abc4b2a76b9719d911017c592" |
      | ={{ 'hello'.hash('sha256') }}                               | "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824" |
      | ={{ 'hello'.base64Encode() }}                               | "aGVsbG8="                |
      | ={{ 'aGVsbG8='.base64Decode() }}                            | "hello"                   |
      | ={{ 'a b&c'.urlEncode() }}                                  | "a%20b%26c"               |

  Scenario Outline: Number extensions
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                  | result |
      | ={{ (1.235).round(2) }}     | 1.24   |
      | ={{ (1.2).ceil() }}         | 2      |
      | ={{ (1.8).floor() }}        | 1      |
      | ={{ (4).isEven() }}         | true   |
      | ={{ (5).isOdd() }}          | true   |
      | ={{ (3).toBoolean() }}      | true   |
      | ={{ (1234.5).format('en-US') }} | "1,234.5" |

  Scenario Outline: Array extensions
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                                          | result            |
      | ={{ [1, 1, 2, 3, 3].unique() }}                     | [1, 2, 3]         |
      | ={{ [{a: 1}, {a: 2}].pluck('a') }}                  | [1, 2]            |
      | ={{ [1, 2, 3].sum() }}                              | 6                 |
      | ={{ [3, 9, 1].max() }}                              | 9                 |
      | ={{ [3, 9, 1].min() }}                              | 1                 |
      | ={{ [2, 4].average() }}                             | 3                 |
      | ={{ [1, 2, 3, 4, 5].chunk(2) }}                     | [[1, 2], [3, 4], [5]] |
      | ={{ [1, 2, 3].first() }}                            | 1                 |
      | ={{ [1, 2, 3].last() }}                             | 3                 |
      | ={{ [1, null, 2, undefined].compact() }}            | [1, 2]            |
      | ={{ [1, 2].isEmpty() }}                             | false             |
      | ={{ [1, 2, 3].difference([2]) }}                    | [1, 3]            |
      | ={{ [1, 2].union([2, 3]) }}                         | [1, 2, 3]         |
      | ={{ [{k: 'a', v: 1}].smartJoin('k', 'v') }}         | {"a": 1}          |

  Scenario Outline: Object extensions
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                                       | result             |
      | ={{ ({a: 1, b: null}).compact() }}               | {"a": 1}           |
      | ={{ ({a: 1, b: 2}).keys() }}                     | ["a", "b"]         |
      | ={{ ({a: 1, b: 2}).values() }}                   | [1, 2]             |
      | ={{ ({a: 1}).hasField('a') }}                    | true               |
      | ={{ ({a: 1, b: 2}).removeField('a') }}           | {"b": 2}           |
      | ={{ ({a: 'x1', b: 'y'}).keepFieldsContaining('1') }} | {"a": "x1"}    |
      | ={{ ({}).isEmpty() }}                            | true               |

  Scenario Outline: Luxon dates
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                                                                                   | result          |
      | ={{ DateTime.fromISO('2024-01-15').toFormat('dd/MM/yyyy') }}                                 | "15/01/2024"    |
      | ={{ DateTime.fromISO('2024-01-31T10:00:00Z').plus({ months: 1 }).toISODate() }}              | "2024-02-29"    |
      | ={{ DateTime.fromISO('2024-03-10T12:00:00', { zone: 'UTC' }).setZone('America/New_York').toFormat('HH:mm') }} | "08:00" |
      | ={{ DateTime.fromISO('2024-01-01').diff(DateTime.fromISO('2023-12-25'), 'days').days }}       | 7               |
      | ={{ '2024-06-01'.toDateTime().toFormat('yyyy') }}                                            | "2024"          |
      | ={{ DateTime.fromISO('2024-02-10').endOf('month').day }}                                     | 29              |
      | ={{ DateTime.fromISO('2024-05-05T08:00:00Z').weekdayLong }}                                  | "Sunday"        |

  Scenario: $now is a Luxon DateTime
    When I evaluate the expression "={{ $now.toISO() }}"
    Then the result is "$datetime"

  Scenario: $today is midnight of the current day
    When I evaluate the expression "={{ [$today.hour, $today.minute, $today.second] }}"
    Then the result is [0, 0, 0]

  Scenario: $now uses the workflow's timezone
    Given the workflow timezone is "Asia/Tokyo"
    When I evaluate the expression "={{ $now.zoneName }}"
    Then the result is "Asia/Tokyo"

  Scenario: Without a workflow timezone $now uses GENERIC_TIMEZONE
    Given the environment variable "GENERIC_TIMEZONE" is "Europe/Berlin"
    When I evaluate the expression "={{ $now.zoneName }}"
    Then the result is "Europe/Berlin"

  Scenario: Dates in items are strings; DateTime objects are serialised as ISO strings
    When I evaluate the expression "={{ DateTime.fromISO('2024-01-15T10:30:00.000Z', { zone: 'utc' }) }}"
    Then the result is "2024-01-15T10:30:00.000Z"
