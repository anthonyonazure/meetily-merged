/** Branding types, mirroring `src-tauri/src/branding/`. */

export interface Branding {
  firm_name: string;
  /** The copy inside the app data directory, never the file originally picked. */
  logo_path: string | null;
  footer_text: string;
  accent_hex: string;
  include_logo: boolean;
  include_footer: boolean;
}

export interface BrandingView extends Branding {
  /** True when a firm name is set, which is what switches branding on. */
  is_configured: boolean;
  /** The stored logo as a data URI, for the picker's preview. */
  logo_data_uri: string | null;
  accent: string;
  accent_ink: string;
}

export interface BrandingInput {
  firm_name: string;
  footer_text: string;
  accent_hex: string;
  include_logo: boolean;
  include_footer: boolean;
}
