#include "jq_component.h"
#include "jq.h"
#include <limits.h>
#include <stdlib.h>
#include <string.h>

static void collect_error(void *data, jv message) {
  jv *errors = data;
  message = jq_format_error(message);
  if (jv_get_kind(message) != JV_KIND_STRING) message = jv_dump_string(message, 0);
  if (jv_string_length_bytes(jv_copy(*errors))) *errors = jv_string_concat(*errors, jv_string("\n"));
  *errors = jv_string_concat(*errors, message);
}
static bool fail(jq_component_list_string_t *out, jq_component_string_t *err, jv message) {
  jq_component_list_string_free(out);
  out->ptr = NULL; out->len = 0;
  if (jv_get_kind(message) != JV_KIND_STRING) message = jv_dump_string(message, 0);
  jq_component_string_dup_n(err, jv_string_value(message), jv_string_length_bytes(jv_copy(message)));
  jv_free(message);
  return false;
}
static bool evaluate(jq_component_string_t *filter, jq_component_string_t *json,
                                   jq_component_list_string_t *out, jq_component_string_t *err) {
  out->ptr = NULL; out->len = 0;
  if (filter->len > INT_MAX || json->len > INT_MAX)
    return fail(out, err, jv_string("input exceeds libjq length limit"));
  if (memchr(filter->ptr, 0, filter->len))
    return fail(out, err, jv_string("filter contains embedded NUL"));
  /* jq 1.8.1 uses %s in jv_parse_sized's error path: supply a terminator. */
  char *json_text = malloc(json->len + 1);
  if (!json_text) abort();
  memcpy(json_text, json->ptr, json->len); json_text[json->len] = 0;
  jv input = jv_parse_sized(json_text, (int)json->len);
  free(json_text);
  if (!jv_is_valid(input)) return fail(out, err, jv_invalid_get_msg(input));
  char *program = malloc(filter->len + 1);
  if (!program) abort();
  memcpy(program, filter->ptr, filter->len); program[filter->len] = 0;
  jq_state *jq = jq_init();
  if (!jq) abort();
  jv errors = jv_string("");
  jq_set_error_cb(jq, collect_error, &errors);
  int compiled = jq_compile(jq, program);
  free(program);
  if (!compiled) {
    jv_free(input); jq_teardown(&jq);
    return fail(out, err, errors);
  }
  jq_start(jq, input, 0);
  size_t capacity = 0;
  jv result;
  while (jv_is_valid(result = jq_next(jq))) {
    jv text = jv_dump_string(result, 0);
    if (out->len == capacity) {
      capacity = capacity ? capacity * 2 : 8;
      void *p = realloc(out->ptr, capacity * sizeof(*out->ptr));
      if (!p) abort();
      out->ptr = p;
    }
    jq_component_string_dup_n(&out->ptr[out->len++], jv_string_value(text), jv_string_length_bytes(jv_copy(text)));
    jv_free(text);
  }
  bool has_error = jv_invalid_has_msg(jv_copy(result));
  jv error = has_error ? jv_invalid_get_msg(result) : (jv_free(result), jv_null());
  if (jq_halted(jq)) {
    jv code = jq_get_exit_code(jq);
    if (jv_get_kind(code) == JV_KIND_NUMBER && jv_number_value(code) != 0) {
      jv_free(error); error = jq_get_error_message(jq);
      if (!jv_is_valid(error)) { jv_free(error); error = jv_string("jq halted with nonzero exit code"); }
      has_error = true;
    }
    jv_free(code);
  }
  jq_teardown(&jq); jv_free(errors);
  if (has_error) return fail(out, err, error);
  jv_free(error);
  return true;
}

bool exports_jq_component_evaluate(jq_component_string_t *filter, jq_component_string_t *json,
                                   jq_component_list_string_t *out, jq_component_string_t *err) {
  bool ok = evaluate(filter, json, out, err);
  /* Exported arguments transfer ownership to the guest. */
  jq_component_string_free(filter);
  jq_component_string_free(json);
  return ok;
}
