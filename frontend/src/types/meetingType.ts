/** Meeting-type types, mirroring `src-tauri/src/meeting_type/`. */

export type MeetingTypeValue =
  | 'discovery'
  | 'status'
  | 'planning'
  | 'incident'
  | 'review'
  | 'one_on_one'
  | 'sales'
  | 'other';

export type TypeSource = 'model' | 'manual';

/** Why a template was chosen. */
export type TemplateChoiceSource =
  | 'client_mapping'
  | 'workspace_mapping'
  | 'requested'
  | 'low_confidence'
  | 'not_classified';

export interface TemplateChoice {
  template_id: string;
  source: TemplateChoiceSource;
  meeting_type: MeetingTypeValue | null;
  confidence: number | null;
}

export interface MeetingTypeOption {
  value: MeetingTypeValue;
  label: string;
  description: string;
}

export interface MeetingTypeView {
  meeting_id: string;
  meeting_type: MeetingTypeValue | null;
  label: string | null;
  confidence: number | null;
  source: TypeSource | null;
  /** True when the classification is trusted enough to pick a template. */
  is_confident: boolean;
  template_choice: TemplateChoice;
  client_id: string | null;
  options: MeetingTypeOption[];
}

export interface TypeTemplateMapping {
  meeting_type: MeetingTypeValue;
  /** Null for the workspace mapping. */
  client_id: string | null;
  template_id: string;
}

export interface MeetingTypeMappings {
  mappings: TypeTemplateMapping[];
  options: MeetingTypeOption[];
  min_confidence: number;
}
