'use client';

/**
 * Settings → Deliverables: what a client-facing export is stamped with.
 *
 * The firm name is the switch. With it empty, exports render exactly as they did
 * before this existed, which is why the panel says so rather than hiding the fields.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Eye, FileText, Image as ImageIcon, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import type { BrandingInput, BrandingView } from '@/types/branding';

const PRESET_ACCENTS = ['#23252b', '#2d5f8b', '#2f5d3a', '#7a4b1f', '#6a3f70', '#a33b36'];

export function DeliverablesPanel() {
  const [view, setView] = useState<BrandingView | null>(null);
  const [firmName, setFirmName] = useState('');
  const [footer, setFooter] = useState('');
  const [accent, setAccent] = useState('#23252b');
  const [includeLogo, setIncludeLogo] = useState(true);
  const [includeFooter, setIncludeFooter] = useState(true);
  const [busy, setBusy] = useState(false);

  const apply = useCallback((loaded: BrandingView) => {
    setView(loaded);
    setFirmName(loaded.firm_name);
    setFooter(loaded.footer_text);
    setAccent(loaded.accent);
    setIncludeLogo(loaded.include_logo);
    setIncludeFooter(loaded.include_footer);
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        apply(await invoke<BrandingView>('branding_get'));
      } catch (error) {
        console.error('Failed to load branding:', error);
      }
    })();
  }, [apply]);

  const save = async () => {
    setBusy(true);
    try {
      const input: BrandingInput = {
        firm_name: firmName,
        footer_text: footer,
        accent_hex: accent,
        include_logo: includeLogo,
        include_footer: includeFooter,
      };
      apply(await invoke<BrandingView>('branding_set', { input }));
      toast.success('Deliverable branding saved');
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const pickLogo = async () => {
    setBusy(true);
    try {
      apply(await invoke<BrandingView>('branding_pick_logo'));
    } catch (error) {
      if (!String(error).includes('cancelled')) toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const clearLogo = async () => {
    setBusy(true);
    try {
      apply(await invoke<BrandingView>('branding_clear_logo'));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const preview = async () => {
    try {
      await invoke('branding_preview');
    } catch (error) {
      toast.error(String(error));
    }
  };

  return (
    <div className="space-y-6 mt-6 max-w-2xl">
      <div className="flex items-start gap-3">
        <FileText className="w-5 h-5 text-muted-ink mt-0.5" />
        <div>
          <h2 className="text-lg font-display font-semibold section-header inline-block">
            Deliverables
          </h2>
          <p className="text-sm text-muted-ink mt-2">
            The header, footer, and accent colour on the exports a client actually reads:
            the PDF (print) and Word formats. Markdown export stays plain, because it is a
            data format rather than a document.
          </p>
        </div>
      </div>

      <div className="bg-surface border border-edge rounded-lg p-4 space-y-4">
        <label className="block text-sm">
          <span className="block text-xs text-muted-ink mb-1">Firm name</span>
          <input
            type="text"
            value={firmName}
            onChange={event => setFirmName(event.target.value)}
            placeholder="Your firm's name"
            className="w-full rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
          />
          <span className="block text-xs text-faint mt-1">
            {view?.is_configured
              ? 'Exports carry your firm name and no marks from this app.'
              : 'While this is empty, exports look exactly as they did before: no header, no footer, no accent.'}
          </span>
        </label>

        {/* Logo */}
        <div className="space-y-2">
          <div className="text-xs text-muted-ink">Logo</div>
          <div className="flex items-center gap-3">
            <div className="w-28 h-16 border border-edge rounded-md bg-wash flex items-center justify-center overflow-hidden">
              {view?.logo_data_uri ? (
                // The data URI comes from the app's own copy of the file.
                // eslint-disable-next-line @next/next/no-img-element
                <img
                  src={view.logo_data_uri}
                  alt="Logo preview"
                  className="max-w-full max-h-full object-contain"
                />
              ) : (
                <ImageIcon className="w-5 h-5 text-faint" />
              )}
            </div>
            <div className="space-y-1">
              <div className="flex items-center gap-2">
                <Button size="sm" variant="outline" onClick={() => void pickLogo()} disabled={busy}>
                  Choose a file
                </Button>
                {view?.logo_path && (
                  <Button size="sm" variant="ghost" onClick={() => void clearLogo()} disabled={busy}>
                    <Trash2 className="w-3.5 h-3.5 mr-1" />
                    Remove
                  </Button>
                )}
              </div>
              <p className="text-xs text-faint max-w-sm">
                PNG, JPEG, GIF, WebP, or SVG, under 2 MB. The file is copied into the
                app&apos;s own storage, so a logo that later moves or is deleted still
                appears on your exports.
              </p>
            </div>
          </div>
          <label className="flex items-center gap-2 text-sm text-ink">
            <input
              type="checkbox"
              checked={includeLogo}
              onChange={event => setIncludeLogo(event.target.checked)}
              className="accent-ink"
            />
            Include the logo
          </label>
          <p className="text-xs text-faint">
            The logo appears on the PDF (print) export. Word documents carry the firm name,
            footer, and accent colour but not the image.
          </p>
        </div>

        {/* Footer */}
        <label className="block text-sm">
          <span className="block text-xs text-muted-ink mb-1">Footer line</span>
          <input
            type="text"
            value={footer}
            onChange={event => setFooter(event.target.value)}
            placeholder="Confidential — prepared for the client"
            className="w-full rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
          />
        </label>
        <label className="flex items-center gap-2 text-sm text-ink">
          <input
            type="checkbox"
            checked={includeFooter}
            onChange={event => setIncludeFooter(event.target.checked)}
            className="accent-ink"
          />
          Include the footer
        </label>

        {/* Accent */}
        <div className="space-y-2">
          <div className="text-xs text-muted-ink">Accent colour</div>
          <div className="flex items-center gap-2 flex-wrap">
            {PRESET_ACCENTS.map(preset => (
              <button
                key={preset}
                onClick={() => setAccent(preset)}
                aria-label={`Use accent ${preset}`}
                className={`w-7 h-7 rounded-md border-2 ${
                  accent.toLowerCase() === preset ? 'border-ink' : 'border-edge'
                }`}
                style={{ backgroundColor: preset }}
              />
            ))}
            <input
              type="text"
              value={accent}
              onChange={event => setAccent(event.target.value)}
              className="w-28 rounded-md border border-edge bg-surface px-2 py-1 text-sm text-ink font-mono"
            />
          </div>
        </div>

        <div className="flex items-center gap-2 pt-1">
          <Button size="sm" onClick={() => void save()} disabled={busy}>
            Save
          </Button>
          <Button size="sm" variant="outline" onClick={() => void preview()} disabled={busy}>
            <Eye className="w-3.5 h-3.5 mr-1.5" />
            Preview a sample export
          </Button>
        </div>
        <p className="text-xs text-faint">
          The preview opens a real export of a sample meeting, produced by the same code
          path your meetings use.
        </p>
      </div>
    </div>
  );
}
