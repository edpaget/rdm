;;; rdm-view.el --- Read-only detail buffers for rdm items  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; Detail buffers for roadmaps, phases and tasks.
;;
;; An rdm item is a plist naming one document:
;;
;;   (:kind roadmap :roadmap SLUG)
;;   (:kind phase   :roadmap SLUG :stem STEM)
;;   (:kind task    :slug SLUG)
;;
;; plus optional display keys (`:title', `:status', `:tags', `:annotation')
;; that pickers fill in.  `rdm-open-item' is the single entry point every
;; picker, embark action, `RET' on a phase row and `rdm-next' goes through:
;; it runs `show --format json' (with `--at' for a historical body) and
;; renders the result in an `rdm-view-mode' buffer, a read-only
;; `special-mode' buffer where `g' re-fetches and `q' quits.  A roadmap
;; view lists its phases as rows carrying an `rdm-item' text property;
;; `RET' opens the row at point.  Evil bindings live in rdm-evil.el.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'rdm-core)

(declare-function markdown-view-mode "ext:markdown-mode")

;;;; Items

(defvar rdm-item--lookup (make-hash-table :test #'equal)
  "Items of the candidates the last picker offered, keyed by string.
Completion frameworks (and embark, which re-reads a candidate through
the minibuffer) may hand a candidate back without its text properties;
this table recovers its item.")

(defun rdm-item-register (candidate item)
  "Return CANDIDATE propertized with ITEM and remember it for lookup."
  (let ((cand (propertize (copy-sequence candidate) 'rdm-item item)))
    (puthash (substring-no-properties candidate) item rdm-item--lookup)
    cand))

(defun rdm-item-from (thing)
  "The rdm item plist THING denotes, or nil.
THING is an item plist, a candidate string carrying an `rdm-item'
property, or a candidate string a picker registered."
  (cond
   ((and (consp thing) (plist-get thing :kind)) thing)
   ((stringp thing)
    (or (and (> (length thing) 0) (get-text-property 0 'rdm-item thing))
        (gethash (substring-no-properties thing) rdm-item--lookup)))))

(defun rdm-item--require (thing)
  "The item THING denotes, or a `user-error'."
  (or (rdm-item-from thing)
      (user-error "rdm: no rdm item for %S" thing)))

(defun rdm-item-identifier (item)
  "The identifier of ITEM: a roadmap or task slug, or ROADMAP/STEM."
  (pcase (plist-get item :kind)
    ('roadmap (plist-get item :roadmap))
    ('phase (concat (plist-get item :roadmap) "/" (plist-get item :stem)))
    ('task (plist-get item :slug))
    (kind (user-error "rdm: no identifier for a %s" kind))))

(defun rdm-item-show-args (item &optional at)
  "The action and `rdm-argv' arguments showing ITEM, at revision AT if non-nil."
  (append
   (pcase (plist-get item :kind)
     ('roadmap (list 'roadmap-show (plist-get item :roadmap)))
     ('phase (list 'phase-show (plist-get item :stem)
                   :roadmap (plist-get item :roadmap)))
     ('task (list 'task-show (plist-get item :slug)))
     (kind (user-error "rdm: cannot show a %s" kind)))
   (when at (list :at at))))

(defun rdm-item-done-line-args (item)
  "The `done-line' `rdm-argv' arguments for ITEM."
  (pcase (plist-get item :kind)
    ('phase (list :roadmap (plist-get item :roadmap)
                  :phase (plist-get item :stem)))
    ('task (list :task (plist-get item :slug)))
    (kind (user-error "rdm: a %s has no Done: line" kind))))

;;;; Buffer state

(defvar-local rdm-view--item nil
  "The item this `rdm-view-mode' buffer shows.")

(defvar-local rdm-view--at nil
  "The revision this buffer's body was read at, or nil for the current one.")

(defface rdm-view-title '((t :inherit bold :height 1.2))
  "Face for the title line of an rdm detail buffer."
  :group 'rdm)

(defface rdm-view-label '((t :inherit font-lock-keyword-face))
  "Face for field labels in an rdm detail buffer."
  :group 'rdm)

(defvar-keymap rdm-view-mode-map
  :doc "Keymap for `rdm-view-mode'.  `g' and `q' come from `special-mode'."
  "RET" #'rdm-view-open-at-point
  "o" #'rdm-view-open-at-point-other-window
  "w" #'rdm-view-copy-identifier
  "d" #'rdm-view-copy-done-line
  "@" #'rdm-view-at)

(define-derived-mode rdm-view-mode special-mode "rdm"
  "Read-only view of one rdm roadmap, phase or task.
\\{rdm-view-mode-map}"
  (setq-local revert-buffer-function #'rdm-view--revert)
  (setq-local truncate-lines nil)
  (require 'rdm-evil))

(defun rdm-view-item-at-point ()
  "The item at point: a phase row's item, else the buffer's item."
  (or (get-text-property (point) 'rdm-item)
      (and (> (point) (point-min))
           (get-text-property (1- (point)) 'rdm-item))
      rdm-view--item))

;;;; Rendering

(defun rdm-view--markdown (body)
  "BODY fontified as Markdown when markdown-mode is available."
  (if (not (require 'markdown-mode nil t))
      body
    (with-temp-buffer
      (insert body)
      (delay-mode-hooks (markdown-view-mode))
      (font-lock-ensure)
      ;; Keep only the faces, as `font-lock-face' so the view buffer's own
      ;; (empty) font-lock does not strip them; markdown-mode's `display'
      ;; and `invisible' markup props would not make sense there.
      (let ((out (buffer-substring-no-properties (point-min) (point-max)))
            (pos (point-min)))
        (while (< pos (point-max))
          (let ((next (next-single-property-change pos 'face nil (point-max)))
                (face (get-text-property pos 'face)))
            (when face
              (put-text-property (1- pos) (1- next) 'font-lock-face face out))
            (setq pos next)))
        out))))

(defun rdm-view--field (label value)
  "Insert a LABEL: VALUE line when VALUE is non-empty."
  (when (and value (not (equal value "")))
    (insert (propertize (concat label ":") 'font-lock-face 'rdm-view-label)
            " " value "\n")))

(defun rdm-view--tags-string (tags)
  "TAGS (a vector or list of strings) joined for display, or nil."
  (when (and tags (> (length tags) 0))
    (mapconcat #'identity tags ", ")))

(defun rdm-view--render (item data at)
  "Render DATA, the `show' JSON of ITEM read at revision AT, into this buffer."
  (let ((inhibit-read-only t)
        (kind (plist-get item :kind)))
    (erase-buffer)
    (insert (propertize (or (plist-get data :title) (rdm-item-identifier item))
                        'font-lock-face 'rdm-view-title)
            "\n")
    (rdm-view--field (capitalize (symbol-name kind)) (rdm-item-identifier item))
    (rdm-view--field "Status" (plist-get data :status))
    (rdm-view--field "Tags" (rdm-view--tags-string (plist-get data :tags)))
    (when at
      (rdm-view--field "Revision" (or (plist-get data :revision) at)))
    (insert "\n")
    (let ((body (or (plist-get data :body) "")))
      (insert (rdm-view--markdown body))
      (unless (or (string-empty-p body) (bolp)) (insert "\n")))
    (when (eq kind 'roadmap)
      (let ((phases (plist-get data :phases))
            (roadmap (plist-get item :roadmap)))
        (insert "\n" (propertize "Phases" 'font-lock-face 'rdm-view-label) "\n")
        (seq-doseq (p phases)
          (let ((phase (list :kind 'phase :roadmap roadmap
                             :stem (plist-get p :stem)
                             :title (plist-get p :title)
                             :status (plist-get p :status))))
            (insert (propertize
                     (format "%3s  %-40s %s"
                             (plist-get p :number)
                             (plist-get p :stem)
                             (or (plist-get p :status) ""))
                     'rdm-item phase)
                    "\n")))))
    (goto-char (point-min))))

(defun rdm-view--buffer-name (item at)
  "The detail buffer name for ITEM at revision AT."
  (format "*rdm %s %s%s*" (plist-get item :kind) (rdm-item-identifier item)
          (if at (concat "@" at) "")))

(defun rdm-view--inherit-context (source)
  "Make this buffer resolve the same rdm context as buffer SOURCE."
  (setq default-directory (buffer-local-value 'default-directory source))
  (dolist (var '(rdm-root rdm-project rdm-bin process-environment))
    (when (local-variable-p var source)
      (set (make-local-variable var) (buffer-local-value var source)))))

(defun rdm-view--fetch (item at)
  "Run ITEM's `show' (at revision AT) in this buffer's context."
  (apply #'rdm-call-sync (rdm-item-show-args item at)))

(defun rdm-view--revert (&rest _)
  "Re-fetch and re-render this buffer's item, keeping point's line."
  (let ((line (line-number-at-pos))
        (data (rdm-view--fetch rdm-view--item rdm-view--at)))
    (rdm-view--render rdm-view--item data rdm-view--at)
    (goto-char (point-min))
    (forward-line (1- line))))

;;;; Commands

;;;###autoload
(defun rdm-open-item (thing &optional at other-window)
  "Show the rdm item THING in a detail buffer.
THING is an item plist or a picker candidate.  AT, when non-nil, is the
revision to read the body at.  OTHER-WINDOW shows it in another window.
Returns the buffer."
  (let* ((item (rdm-item--require thing))
         (source (current-buffer))
         (data (rdm-view--fetch item at))
         (buf (get-buffer-create (rdm-view--buffer-name item at))))
    (with-current-buffer buf
      (unless (derived-mode-p 'rdm-view-mode)
        (rdm-view-mode))
      (unless (eq source buf)
        (rdm-view--inherit-context source))
      (setq rdm-view--item item
            rdm-view--at at)
      (rdm-view--render item data at))
    (if other-window
        (switch-to-buffer-other-window buf)
      (pop-to-buffer-same-window buf))
    buf))

;;;###autoload
(defun rdm-open-item-other-window (thing)
  "Show the rdm item THING in a detail buffer in another window."
  (interactive (list (rdm-view-item-at-point)))
  (rdm-open-item thing nil t))

(defun rdm-view-open-at-point ()
  "Open the item at point (a phase row, or this buffer's item)."
  (interactive)
  (rdm-open-item (or (get-text-property (point) 'rdm-item)
                     (user-error "rdm: no item at point"))))

(defun rdm-view-open-at-point-other-window ()
  "Open the item at point in another window."
  (interactive)
  (rdm-open-item (or (get-text-property (point) 'rdm-item)
                     (user-error "rdm: no item at point"))
                 nil t))

(defun rdm-view-at (revision &optional thing)
  "Show the item at point (or THING) with its body read at REVISION."
  (interactive
   (let ((item (rdm-view-item-at-point)))
     (unless item (user-error "rdm: not in an rdm detail buffer"))
     (list (read-string (format "Revision of %s (default HEAD): "
                                (rdm-item-identifier item))
                        nil nil "HEAD")
           item)))
  (rdm-open-item (or thing (rdm-view-item-at-point)
                     (user-error "rdm: not in an rdm detail buffer"))
                 revision))

(defun rdm-copy-identifier (thing)
  "Copy the identifier of the rdm item THING to the kill ring; return it."
  (let ((id (rdm-item-identifier (rdm-item--require thing))))
    (kill-new id)
    (message "Copied %s" id)
    id))

(defun rdm-copy-done-line (thing)
  "Copy the `Done:' commit line for the rdm item THING; return it."
  (let* ((item (rdm-item--require thing))
         (line (string-trim
                (apply #'rdm-call-sync 'done-line
                       (rdm-item-done-line-args item)))))
    (kill-new line)
    (message "Copied %s" line)
    line))

(defun rdm-view-copy-identifier ()
  "Copy the identifier of the item at point."
  (interactive)
  (rdm-copy-identifier (rdm-view-item-at-point)))

(defun rdm-view-copy-done-line ()
  "Copy the `Done:' commit line of the item at point."
  (interactive)
  (rdm-copy-done-line (rdm-view-item-at-point)))

(provide 'rdm-view)
;;; rdm-view.el ends here
