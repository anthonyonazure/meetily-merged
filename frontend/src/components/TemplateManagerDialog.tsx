"use client";

import { useState, useCallback } from 'react';
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Label } from '@/components/ui/label';
import { Separator } from '@/components/ui/separator';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Plus, Trash2, Eye, Pencil, ArrowLeft, X, Copy } from 'lucide-react';
import { toast } from 'sonner';
import type { TemplateInfo, TemplateDetails, TemplateSectionInfo } from '@/hooks/meeting-details/useTemplates';

interface TemplateManagerDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  templates: TemplateInfo[];
  onFetchDetails: (templateId: string) => Promise<TemplateDetails>;
  onSave: (templateId: string, data: { name: string; description: string; sections: TemplateSectionInfo[] }) => Promise<void>;
  onDelete: (templateId: string) => Promise<void>;
}

type EditorMode = 'view' | 'edit' | 'create';

const EMPTY_SECTION: TemplateSectionInfo = {
  title: '',
  instruction: '',
  format: 'paragraph',
};

export function TemplateManagerDialog({
  open,
  onOpenChange,
  templates,
  onFetchDetails,
  onSave,
  onDelete,
}: TemplateManagerDialogProps) {
  const [view, setView] = useState<'list' | 'editor'>('list');
  const [editorMode, setEditorMode] = useState<EditorMode>('view');
  const [loading, setLoading] = useState(false);

  // Editor state
  const [editId, setEditId] = useState('');
  const [editName, setEditName] = useState('');
  const [editDescription, setEditDescription] = useState('');
  const [editSections, setEditSections] = useState<TemplateSectionInfo[]>([]);
  const [isCustom, setIsCustom] = useState(false);

  const resetEditor = useCallback(() => {
    setEditId('');
    setEditName('');
    setEditDescription('');
    setEditSections([]);
    setIsCustom(false);
  }, []);

  const goToList = useCallback(() => {
    setView('list');
    resetEditor();
  }, [resetEditor]);

  const openEditor = useCallback(async (templateId: string, mode: EditorMode) => {
    setLoading(true);
    try {
      const details = await onFetchDetails(templateId);
      setEditId(details.id);
      setEditName(details.name);
      setEditDescription(details.description);
      setEditSections(details.sections);
      setIsCustom(details.is_custom);
      setEditorMode(mode);
      setView('editor');
    } catch (error) {
      toast.error('Failed to load template details', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setLoading(false);
    }
  }, [onFetchDetails]);

  const handleDuplicate = useCallback(async (templateId: string) => {
    setLoading(true);
    try {
      const details = await onFetchDetails(templateId);
      setEditId('');
      setEditName(details.name + ' (Copy)');
      setEditDescription(details.description);
      setEditSections(details.sections);
      setIsCustom(true);
      setEditorMode('create');
      setView('editor');
    } catch (error) {
      toast.error('Failed to load template', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setLoading(false);
    }
  }, [onFetchDetails]);

  const handleCreate = useCallback(() => {
    resetEditor();
    setEditSections([{ ...EMPTY_SECTION }]);
    setIsCustom(true);
    setEditorMode('create');
    setView('editor');
  }, [resetEditor]);

  const handleDelete = useCallback(async (templateId: string) => {
    try {
      await onDelete(templateId);
    } catch (error) {
      toast.error('Failed to delete template', {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  }, [onDelete]);

  const handleSave = useCallback(async () => {
    let templateId = editId;
    if (editorMode === 'create') {
      templateId = editName
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '_')
        .replace(/^_|_$/g, '');
      if (!templateId) {
        toast.error('Invalid template name');
        return;
      }
    }

    setLoading(true);
    try {
      await onSave(templateId, {
        name: editName,
        description: editDescription,
        sections: editSections,
      });
      goToList();
    } catch (error) {
      toast.error('Failed to save template', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setLoading(false);
    }
  }, [editId, editName, editDescription, editSections, editorMode, onSave, goToList]);

  const updateSection = useCallback((index: number, field: keyof TemplateSectionInfo, value: string) => {
    setEditSections(prev => {
      const updated = [...prev];
      updated[index] = { ...updated[index], [field]: value || undefined };
      if (field === 'format') {
        updated[index].format = value;
      }
      return updated;
    });
  }, []);

  const addSection = useCallback(() => {
    setEditSections(prev => [...prev, { ...EMPTY_SECTION }]);
  }, []);

  const removeSection = useCallback((index: number) => {
    setEditSections(prev => prev.filter((_, i) => i !== index));
  }, []);

  const isReadOnly = editorMode === 'view';
  const canSave = editorMode !== 'view' && editName.trim() && editDescription.trim() && editSections.length > 0 &&
    editSections.every(s => s.title.trim() && s.instruction.trim() && s.format);

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) { goToList(); } onOpenChange(v); }}>
      <DialogContent
        className="max-w-2xl !flex !flex-col overflow-hidden"
        style={{ maxHeight: '80vh' }}
        aria-describedby={undefined}
      >
        <DialogTitle className="flex-shrink-0">
          {view === 'list' ? 'Manage Templates' : (
            editorMode === 'create' ? 'New Template' :
            editorMode === 'edit' ? `Edit: ${editName}` :
            editName
          )}
        </DialogTitle>

        {view === 'list' ? (
          /* List View */
          <div className="flex-1 overflow-y-auto min-h-0 -mx-6 px-6">
            <div className="space-y-2">
              {templates.map((template) => (
                <div
                  key={template.id}
                  className="flex items-center justify-between p-3 rounded-lg border hover:bg-gray-50"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-medium text-sm truncate">{template.name}</span>
                      {template.is_custom && (
                        <span className="text-xs bg-blue-100 text-blue-700 px-1.5 py-0.5 rounded flex-shrink-0">Custom</span>
                      )}
                    </div>
                    <p className="text-xs text-gray-500 truncate">{template.description}</p>
                  </div>
                  <div className="flex items-center gap-1 ml-2 flex-shrink-0">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => openEditor(template.id, 'view')}
                      disabled={loading}
                      title="View template"
                    >
                      <Eye className="h-4 w-4" />
                    </Button>
                    {template.is_custom ? (
                      <>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => openEditor(template.id, 'edit')}
                          disabled={loading}
                          title="Edit template"
                        >
                          <Pencil className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleDelete(template.id)}
                          disabled={loading}
                          title="Delete template"
                          className="text-red-500 hover:text-red-700 hover:bg-red-50"
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </>
                    ) : (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDuplicate(template.id)}
                        disabled={loading}
                        title="Duplicate as custom template"
                      >
                        <Copy className="h-4 w-4" />
                      </Button>
                    )}
                  </div>
                </div>
              ))}
            </div>
            <Separator className="my-4" />
            <Button
              variant="outline"
              className="w-full mb-2"
              onClick={handleCreate}
            >
              <Plus className="h-4 w-4 mr-2" />
              New Template
            </Button>
          </div>
        ) : (
          /* Editor View */
          <>
            <div className="flex items-center gap-2 flex-shrink-0 -mt-2">
              <Button variant="ghost" size="sm" onClick={goToList}>
                <ArrowLeft className="h-4 w-4 mr-1" />
                Back
              </Button>
              {isReadOnly && isCustom && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setEditorMode('edit')}
                >
                  <Pencil className="h-4 w-4 mr-1" />
                  Edit
                </Button>
              )}
              {isReadOnly && !isCustom && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleDuplicate(editId)}
                >
                  <Copy className="h-4 w-4 mr-1" />
                  Duplicate
                </Button>
              )}
            </div>

            <div className="flex-1 overflow-y-auto min-h-0 -mx-6 px-6">
              <div className="space-y-4 pb-4">
                {/* Name */}
                <div className="space-y-1.5">
                  <Label htmlFor="template-name">Name</Label>
                  <Input
                    id="template-name"
                    value={editName}
                    onChange={(e) => setEditName(e.target.value)}
                    disabled={isReadOnly}
                    placeholder="My Meeting Template"
                  />
                </div>

                {/* Description */}
                <div className="space-y-1.5">
                  <Label htmlFor="template-description">Description</Label>
                  <Input
                    id="template-description"
                    value={editDescription}
                    onChange={(e) => setEditDescription(e.target.value)}
                    disabled={isReadOnly}
                    placeholder="A brief description of this template"
                  />
                </div>

                <Separator />

                {/* Sections */}
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <Label>Sections</Label>
                    {!isReadOnly && (
                      <Button variant="outline" size="sm" onClick={addSection}>
                        <Plus className="h-3 w-3 mr-1" />
                        Add Section
                      </Button>
                    )}
                  </div>

                  {editSections.map((section, index) => (
                    <div key={index} className="border rounded-lg p-3 space-y-3 bg-gray-50/50">
                      <div className="flex items-center justify-between">
                        <span className="text-xs font-medium text-gray-500">Section {index + 1}</span>
                        {!isReadOnly && editSections.length > 1 && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => removeSection(index)}
                            className="h-6 w-6 p-0 text-red-400 hover:text-red-600"
                          >
                            <X className="h-3 w-3" />
                          </Button>
                        )}
                      </div>

                      <div className="space-y-1">
                        <Label className="text-xs">Title</Label>
                        <Input
                          value={section.title}
                          onChange={(e) => updateSection(index, 'title', e.target.value)}
                          disabled={isReadOnly}
                          placeholder="e.g., Action Items"
                          className="h-8 text-sm"
                        />
                      </div>

                      <div className="space-y-1">
                        <Label className="text-xs">Instruction</Label>
                        <Textarea
                          value={section.instruction}
                          onChange={(e) => updateSection(index, 'instruction', e.target.value)}
                          disabled={isReadOnly}
                          placeholder="Instructions for the AI on what to extract..."
                          className="text-sm min-h-[60px]"
                        />
                      </div>

                      <div className="space-y-1">
                        <Label className="text-xs">Format</Label>
                        <Select
                          value={section.format}
                          onValueChange={(value) => updateSection(index, 'format', value)}
                          disabled={isReadOnly}
                        >
                          <SelectTrigger className="h-8 text-sm">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="paragraph">Paragraph</SelectItem>
                            <SelectItem value="list">List</SelectItem>
                            <SelectItem value="string">String</SelectItem>
                          </SelectContent>
                        </Select>
                      </div>

                      <div className="space-y-1">
                        <Label className="text-xs">Item Format (optional)</Label>
                        <Input
                          value={section.item_format || ''}
                          onChange={(e) => updateSection(index, 'item_format', e.target.value)}
                          disabled={isReadOnly}
                          placeholder="e.g., **[Owner]**: [Task] (Due: [Date])"
                          className="h-8 text-sm"
                        />
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>

            {/* Save/Cancel buttons */}
            {!isReadOnly && (
              <div className="flex justify-end gap-2 pt-2 border-t flex-shrink-0">
                <Button variant="outline" size="sm" onClick={goToList}>
                  Cancel
                </Button>
                <Button
                  size="sm"
                  onClick={handleSave}
                  disabled={!canSave || loading}
                >
                  {loading ? 'Saving...' : 'Save Template'}
                </Button>
              </div>
            )}
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
