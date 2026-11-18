import protobuf from "protobufjs";

function reject() {
  throw Object.assign(new TypeError("invalid source query request"), { status: 400 });
}

/** @param {protobuf.Type} type @param {unknown} input */
export function validateSourceRequest(type, input) {
  let work = 0;
  /** @param {protobuf.Type} message @param {unknown} value @param {number} depth */
  function visit(message, value, depth) {
    if (++work > 10000 || depth > 32 || value === null || typeof value !== "object" || Array.isArray(value)) reject();
    message.resolveAll();
    const record = /** @type {Record<string, unknown>} */ (value);
    if (Object.keys(record).some((key) => !Object.hasOwn(message.fields, key))) reject();
    for (const oneof of message.oneofsArray) {
      const present = oneof.oneof.filter((key) => Object.hasOwn(record, key));
      if (present.length > 1 || (oneof.name === "selection" && present.length !== 1)) reject();
    }
    for (const [key, item] of Object.entries(record)) {
      const field = message.fields[key];
      if (field.map) reject();
      if (field.repeated) {
        if (!Array.isArray(item) || item.length > 500) reject();
        for (const entry of /** @type {unknown[]} */ (item)) scalar(field, entry, depth);
      } else scalar(field, item, depth);
    }
  }
  /** @param {protobuf.Field} field @param {unknown} value @param {number} depth */
  function scalar(field, value, depth) {
    if (++work > 10000) reject();
    if (field.resolvedType instanceof protobuf.Type) return visit(field.resolvedType, value, depth + 1);
    if (field.resolvedType instanceof protobuf.Enum) {
      const values = field.resolvedType.values;
      if (!((typeof value === "string" && Object.hasOwn(values, value)) || (typeof value === "number" && Number.isInteger(value) && Object.values(values).includes(value)))) reject();
    } else if (field.type === "string") {
      if (typeof value !== "string" || Buffer.byteLength(value) > 16384) reject();
    } else if (field.type === "bool") {
      if (typeof value !== "boolean") reject();
    } else if (["uint32", "fixed32", "int32", "sint32", "sfixed32"].includes(field.type)) {
      const unsigned = field.type === "uint32" || field.type === "fixed32";
      if (typeof value !== "number" || !Number.isInteger(value) || value < (unsigned ? 0 : -2147483648) || value > (unsigned ? 4294967295 : 2147483647)) reject();
    } else if (["uint64", "fixed64", "int64", "sint64", "sfixed64"].includes(field.type)) {
      const unsigned = field.type === "uint64" || field.type === "fixed64";
      if (!((typeof value === "number" && Number.isSafeInteger(value)) || (typeof value === "string" && /^-?[0-9]+$/.test(value)))) reject();
      const number = BigInt(/** @type {string | number} */ (value));
      if (number < (unsigned ? 0n : -9223372036854775808n) || number > (unsigned ? 18446744073709551615n : 9223372036854775807n)) reject();
    } else reject();
  }
  visit(type, input, 1);
}
