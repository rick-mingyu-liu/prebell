/**
 * A small JSON Schema check for the keywords the MCP tool schemas use (type, enum, pattern, properties, required,
 * additionalProperties: false, items, maxItems). Returns the problems found, empty when `value` conforms.
 */
type Schema = Record<string, unknown>;

const typeOf = (v: unknown): string =>
  v === null ? "null" : Array.isArray(v) ? "array" : Number.isInteger(v) ? "integer" : typeof v;

export function schemaErrors(schema: Schema, value: unknown, path = "$"): string[] {
  const errors: string[] = [];
  if (schema.type !== undefined) {
    const types = (Array.isArray(schema.type) ? schema.type : [schema.type]) as string[];
    const t = typeOf(value);
    if (!types.some((x) => x === t || (x === "number" && t === "integer")))
      return [`${path}: expected ${types.join("|")}, got ${t}`];
  }
  if (Array.isArray(schema.enum) && !schema.enum.includes(value)) errors.push(`${path}: ${String(value)} not in enum`);
  if (typeof schema.pattern === "string" && typeof value === "string" && !new RegExp(schema.pattern).test(value))
    errors.push(`${path}: does not match ${schema.pattern}`);
  if (typeOf(value) === "object") {
    const obj = value as Record<string, unknown>;
    const props = (schema.properties ?? {}) as Record<string, Schema>;
    for (const r of (schema.required ?? []) as string[]) if (!(r in obj)) errors.push(`${path}: missing ${r}`);
    for (const [k, v] of Object.entries(obj)) {
      if (props[k]) errors.push(...schemaErrors(props[k], v, `${path}.${k}`));
      else if (schema.additionalProperties === false) errors.push(`${path}: unexpected ${k}`);
    }
  }
  if (Array.isArray(value)) {
    if (typeof schema.maxItems === "number" && value.length > schema.maxItems) errors.push(`${path}: too many items`);
    if (schema.items) value.forEach((v, i) => errors.push(...schemaErrors(schema.items as Schema, v, `${path}[${i}]`)));
  }
  return errors;
}
