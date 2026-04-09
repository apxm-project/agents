export type OpFieldSpec = {
  name: string;
  required: boolean;
  description: string;
  ref_type: string | null;
};

export type OpSpec = {
  op_type: string;
  name: string;
  category: string;
  description: string;
  long_description: string;
  latency: string;
  example_json: string | null;
  fields: OpFieldSpec[];
  needs_submission: boolean;
  min_inputs: number;
  produces_output: boolean;
};
