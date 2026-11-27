# HTML cleaning (rinch-html)

`rinch_html::clean` runs a document through these steps, in order (`crates/rinch-html/src/clean.rs`, `clean_inner`):

1. `depth::cap`: cuts the nesting to `MAX_DEPTH` (512) before anything recursive reads it.
2. `user_sheet::mark`: marks elements for the app's user sheet, after stripping marks the document wrote.
3. `media::resolve`: resolves `@media` blocks.
4. `inline_sheets`: inlines `<style>` sheets.
5. Ammonia, under rinch's allow-list, narrowed by `Policy::allow` (`allow.rs`, `AllowList::narrow`) when set.
6. `allow::carry`: renames carried attributes, only when `Policy::allow` carries any.

## Carrying runs after the cleaning, never before it

A carried attribute, such as OFSCP's `img[data-attachment]`, is kept as itself through ammonia, which is told to keep it on the elements named. A pass over ammonia's output then renames it to `CARRIED` + its name. That pass parses the output as a fragment in a `div`, the way ammonia parses its input, and writes it back with `depth::write_children`. Ammonia drops any attribute the document wrote under a carried name, as it drops any attribute it wasn't told to keep. So in the output, only a carried value has such a name.

The rename must not run before ammonia. HTML doesn't always parse the same way twice: `<form><math><mtext></form><form><mglyph><style></math>…` is text on one parse and elements on the next. A strip-and-rename pre-pass works on one parse, and ammonia then parses the written-out document again, so an element the pre-pass saw only as text reaches ammonia with its forged attribute (rinch#3094 review). The user-sheet marks in step 2 have the same flaw (#3095).

`rel` can't be carried. Rinch sets `link_rel` on every link, and ammonia asserts that `rel` isn't also an allowed attribute, so `AllowList::carried_attributes` skips it (`WRITTEN_BY_THE_CLEANER`).

## `<pre>` and re-serializing

The parser drops one newline straight after `<pre>`, `<textarea>` and `<listing>`. html5ever's serializer doesn't write it back, so each parse and write would cost a text one leading newline. `depth`'s serializer (`Children::serialize`) writes an extra `\n` when such an element's text starts with one, so `cap` and `carry` are lossless. Ammonia's own serializer still drops one, as it always has. Pins: `depth::tests::a_preformatted_texts_leading_newline_survives_a_write`, `tests/allow.rs` `carrying_does_not_cost_a_preformatted_text_its_leading_newline`.
