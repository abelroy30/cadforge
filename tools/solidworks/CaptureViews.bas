Attribute VB_Name = "CaptureViews"
'=====================================================================
' CaptureViews - screenshot the open SolidWorks model from several angles.
'
' WHY THIS EXISTS
'   The vision side of cadforge learns from pictures of finished parts.
'   Rendering four views in the browser works with any file and needs no
'   licence, but a picture of what SolidWorks itself draws - the shaded
'   model, with its real edges and colours - is the closer thing to what
'   you would hand a person to work from. This macro takes those pictures.
'
' HOW TO INSTALL
'   1. In SolidWorks: Tools > Macro > New, save as CaptureViews.swp.
'      (Or paste this into the VBA editor: Tools > Macro > Edit.)
'   2. Open the part or assembly whose views you want.
'   3. Run the macro (Tools > Macro > Run).
'   4. It writes <DocumentName>-front.png, -top.png, -right.png, -iso.png
'      beside the model file. Drop them onto the Images card in cadforge.
'
' NAMING - THIS IS THE PART THAT MATTERS
'   cadforge pairs a picture with a document by name: strip the extension,
'   then strip a trailing view word, and `bracket-front.png` becomes
'   `bracket`, which is the `bracket.SLDPRT` it belongs to. So the
'   suffixes written here are exactly the words the studio knows - `iso`,
'   not `isometric` - and a renamed file would stop pairing.
'
' OPTIONAL BULK RUN
'   Put a folder in SWBATCH_ROOT and the macro walks every part and
'   assembly under it, capturing each one. Leave it empty to capture just
'   the open document.
'
' REQUIRES
'   SolidWorks exports a raster image when a part is saved with an image
'   extension, which is what this uses. If a capture fails, the macro says
'   so rather than writing a broken file; the usual cause is that image
'   export is switched off, and File > Save As, choosing PNG by hand,
'   will show whether it is available at all.
'=====================================================================
Option Explicit

' Leave empty to capture just the open document.
Private Const SWBATCH_ROOT As String = ""

' Leave empty to write the pictures next to the model file.
Private Const OUT_FOLDER As String = ""

' Shaded, because a wireframe is hard to read and harder to learn from.
Private Const SHADED As Boolean = True

' The documented swStandardViews_e values. The name is what actually
' selects the view; the id is SolidWorks' own handle for it.
Private Const SW_VIEW_FRONT   As Long = 1
Private Const SW_VIEW_BACK    As Long = 2
Private Const SW_VIEW_LEFT    As Long = 3
Private Const SW_VIEW_RIGHT   As Long = 4
Private Const SW_VIEW_TOP     As Long = 5
Private Const SW_VIEW_BOTTOM  As Long = 6
Private Const SW_VIEW_ISO     As Long = 7

'---------------------------------------------------------------- model

Public Sub main()

    Dim swApp As SldWorks.SldWorks
    Set swApp = Application.SldWorks

    If Len(SWBATCH_ROOT) > 0 Then
        CaptureFolder swApp, SWBATCH_ROOT
        MsgBox "Captured views for every model under " & vbNewLine & SWBATCH_ROOT, vbInformation
        Exit Sub
    End If

    Dim doc As SldWorks.ModelDoc2
    Set doc = swApp.ActiveDoc
    If doc Is Nothing Then
        MsgBox "Open a part or assembly first.", vbExclamation
        Exit Sub
    End If

    Dim written As Long
    written = CaptureViewsOf(doc)

    If written = 0 Then
        MsgBox "No picture was written for " & FileNameOf(doc) & "." & vbNewLine & vbNewLine & _
               "Check that SolidWorks can save an image: File > Save As and look for " & _
               "PNG in the type list.", vbExclamation
    Else
        MsgBox "Wrote " & CStr(written) & " view(s) for " & vbNewLine & FileNameOf(doc) & _
               vbNewLine & vbNewLine & "Drop them onto the Images card in cadforge.", vbInformation
    End If
End Sub

'---------------------------------------------------------------- batch

Private Sub CaptureFolder(swApp As SldWorks.SldWorks, root As String)

    Dim fso As Object
    Set fso = CreateObject("Scripting.FileSystemObject")
    If Not fso.FolderExists(root) Then
        MsgBox "No such folder: " & root, vbExclamation
        Exit Sub
    End If

    Dim folder As Object, file As Object, sub_ As Object
    Dim ext As String, doc As SldWorks.ModelDoc2, total As Long

    Set folder = fso.GetFolder(root)
    For Each file In folder.Files
        ext = LCase$(fso.GetExtensionName(file.Name))
        If ext = "sldprt" Or ext = "sldasm" Then
            Set doc = swApp.OpenDoc6(file.Path, DocTypeFor(ext), 1, "", 0, 0)
            If Not doc Is Nothing Then
                total = total + CaptureViewsOf(doc)
                swApp.CloseDoc doc.GetTitle
            End If
        End If
    Next file

    ' Recurse into subfolders.
    For Each sub_ In folder.SubFolders
        CaptureFolder swApp, sub_.Path
    Next sub_
End Sub

Private Function DocTypeFor(ext As String) As Long
    ' swDocPART = 1, swDocASSEMBLY = 2
    If ext = "sldasm" Then DocTypeFor = 2 Else DocTypeFor = 1
End Function

'---------------------------------------------------------------- capture

' Sets each view in turn and saves it as a picture. Returns how many were
' actually written, so a silent failure to export is visible to the caller
' instead of looking like success over a folder of nothing.
Private Function CaptureViewsOf(doc As SldWorks.ModelDoc2) As Long

    Dim outDir As String
    outDir = OutFolderFor(doc)

    Dim fso As Object
    Set fso = CreateObject("Scripting.FileSystemObject")
    If Len(outDir) > 0 And Not fso.FolderExists(outDir) Then
        fso.CreateFolder outDir
    End If

    Dim stem As String
    stem = fso.GetBaseName(FileNameOf(doc))

    Dim written As Long
    written = 0

    ' Four views by default. Uncomment the rest for a rounder survey - the
    ' studio accepts as many as you give it, and more views per part is
    ' simply more training data.
    written = written + CaptureOne(doc, fso.BuildPath(outDir, stem & "-front.png"), "*Front", SW_VIEW_FRONT)
    written = written + CaptureOne(doc, fso.BuildPath(outDir, stem & "-top.png"), "*Top", SW_VIEW_TOP)
    written = written + CaptureOne(doc, fso.BuildPath(outDir, stem & "-right.png"), "*Right", SW_VIEW_RIGHT)
    written = written + CaptureOne(doc, fso.BuildPath(outDir, stem & "-iso.png"), "*Isometric", SW_VIEW_ISO)
    ' written = written + CaptureOne(doc, fso.BuildPath(outDir, stem & "-back.png"), "*Back", SW_VIEW_BACK)
    ' written = written + CaptureOne(doc, fso.BuildPath(outDir, stem & "-left.png"), "*Left", SW_VIEW_LEFT)
    ' written = written + CaptureOne(doc, fso.BuildPath(outDir, stem & "-bottom.png"), "*Bottom", SW_VIEW_BOTTOM)

    CaptureViewsOf = written
End Function

' One view, one file. Returns 1 if a picture of a non-zero size landed on
' disk, 0 otherwise - a zero-byte file counts as a failure, because that is
' what an unavailable exporter leaves behind.
Private Function CaptureOne(doc As SldWorks.ModelDoc2, ByVal path As String, _
                            ByVal viewName As String, ByVal viewId As Long) As Long

    On Error GoTo Failed

    ' Put the model in the view, framed and filled, before photographing it.
    doc.ShowNamedView2 viewName, viewId
    doc.ViewZoomtofit2
    If SHADED Then doc.ViewDisplayShaded

    ' Saving a part under an image extension exports the graphics area.
    ' swSaveAsOptions_Silent (1) keeps the macro from stopping on a dialog.
    doc.SaveAs3 path, 0, 1

    ' Do not trust the return value; trust the file on disk.
    If FileSizeOf(path) > 0 Then
        CaptureOne = 1
    Else
        CaptureOne = 0
    End If
    Exit Function

Failed:
    CaptureOne = 0
End Function

'---------------------------------------------------------------- helpers

Private Function FileSizeOf(ByVal path As String) As Long
    Dim fso As Object
    Set fso = CreateObject("Scripting.FileSystemObject")
    If fso.FileExists(path) Then
        FileSizeOf = fso.GetFile(path).Size
    Else
        FileSizeOf = 0
    End If
End Function

Private Function OutFolderFor(doc As SldWorks.ModelDoc2) As String
    If Len(OUT_FOLDER) > 0 Then
        OutFolderFor = OUT_FOLDER
        Exit Function
    End If
    Dim p As String
    p = doc.GetPathName
    If Len(p) = 0 Then
        ' An unsaved document has no folder to write beside, so use the profile.
        OutFolderFor = Environ$("USERPROFILE")
    Else
        OutFolderFor = Left$(p, InStrRev(p, "\") - 1)
    End If
End Function

Private Function FileNameOf(doc As SldWorks.ModelDoc2) As String
    Dim p As String
    p = doc.GetPathName
    If Len(p) = 0 Then
        FileNameOf = doc.GetTitle
    Else
        FileNameOf = Mid$(p, InStrRev(p, "\") + 1)
    End If
End Function
